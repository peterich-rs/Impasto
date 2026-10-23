use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use protocol::DrawList;
use session::Lifecycle;

use crate::command::Command;
use crate::error::BootError;
use crate::input::{Input, same_draw_list};
use crate::present::{AfterPresent, after_present};
use crate::queue::Inbox;
use crate::shaper::ImpellerShaper;
use crate::{Boot, Launch, LogLevel, Surface};

/// Inputs to one present decision. `paused` comes from [`Input`], not a second flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameView {
    pub surface_open: bool,
    pub paused: bool,
    pub fixture_open: bool,
    pub fixture_presented: bool,
    pub unchanged: bool,
    pub needs_present: bool,
    pub again: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameStep {
    Cancel,
    Skip { again: bool },
    Submit,
    PresentCached,
}

/// What to do after [`after_present`], without touching the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentFollowup {
    pub needs_present: bool,
    pub mark_fixture_presented: bool,
    pub clear_surface: bool,
    pub vsync: VsyncOp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VsyncOp {
    Request,
    Cancel,
    Stop,
}

pub(crate) fn frame_step(view: &FrameView) -> FrameStep {
    if !view.surface_open || view.paused || (view.fixture_open && view.fixture_presented) {
        return FrameStep::Cancel;
    }
    if view.unchanged && !view.needs_present {
        return FrameStep::Skip { again: view.again };
    }
    if view.unchanged {
        FrameStep::PresentCached
    } else {
        FrameStep::Submit
    }
}

pub(crate) fn present_followup(after: AfterPresent, is_fixture: bool) -> PresentFollowup {
    match after {
        AfterPresent::Shown { again } => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: is_fixture,
            clear_surface: false,
            vsync: if again {
                VsyncOp::Request
            } else {
                VsyncOp::Cancel
            },
        },
        AfterPresent::Stop => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: false,
            clear_surface: false,
            vsync: VsyncOp::Stop,
        },
        AfterPresent::Retry => PresentFollowup {
            needs_present: true,
            mark_fixture_presented: false,
            clear_surface: false,
            vsync: VsyncOp::Request,
        },
        AfterPresent::WaitForSurface => PresentFollowup {
            needs_present: false,
            mark_fixture_presented: false,
            clear_surface: true,
            vsync: VsyncOp::Cancel,
        },
    }
}

/// Surface loss clears the last list. `SurfaceDestroyed` and `WaitForSurface` share this.
pub(crate) fn surface_lost() -> PresentFollowup {
    PresentFollowup {
        needs_present: false,
        mark_fixture_presented: false,
        clear_surface: true,
        vsync: VsyncOp::Cancel,
    }
}

pub struct EngineSlot {
    engine: Rc<RefCell<engine::Engine>>,
    log: Rc<dyn Fn(LogLevel, &str)>,
}

impl EngineSlot {
    pub fn shaper(&self) -> ImpellerShaper {
        ImpellerShaper::new(Rc::clone(&self.engine))
    }

    pub fn log_hook(&self) -> Box<dyn Fn(u8, &str)> {
        let log = Rc::clone(&self.log);
        Box::new(move |level, message| {
            let level = if level == 3 {
                LogLevel::Error
            } else {
                LogLevel::Info
            };
            log(level, message);
        })
    }
}

#[derive(Clone)]
pub struct Poster {
    inbox: Arc<Inbox>,
}

impl Poster {
    pub fn post(&self, command: Command) {
        self.inbox.post(command);
    }
}

pub struct Link {
    inbox: Arc<Inbox>,
    done: Mutex<Option<Receiver<Result<(), BootError>>>>,
    surface_dropped: Arc<Ack>,
}

impl Link {
    pub fn poster(&self) -> Poster {
        Poster {
            inbox: Arc::clone(&self.inbox),
        }
    }

    pub fn post(&self, command: Command) {
        self.inbox.post(command);
    }

    pub fn destroy_surface_and_wait(&self) {
        self.surface_dropped.reset();
        self.inbox.post(Command::SurfaceDestroyed);
        self.surface_dropped.wait();
    }

    pub fn shutdown_and_join(&self) -> Result<(), BootError> {
        self.inbox.post(Command::Shutdown);
        self.join()
    }

    fn join(&self) -> Result<(), BootError> {
        let receiver = lock(&self.done).take();
        match receiver {
            Some(receiver) => match receiver.recv() {
                Ok(result) => result,
                Err(_) => Ok(()),
            },
            None => Ok(()),
        }
    }
}

struct Ack {
    ready: Mutex<bool>,
    cv: Condvar,
}

impl Ack {
    fn new() -> Self {
        Self {
            ready: Mutex::new(false),
            cv: Condvar::new(),
        }
    }

    fn reset(&self) {
        *lock(&self.ready) = false;
    }

    fn signal(&self) {
        *lock(&self.ready) = true;
        self.cv.notify_one();
    }

    fn wait(&self) {
        let mut ready = lock(&self.ready);
        while !*ready {
            ready = self
                .cv
                .wait(ready)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

pub fn start(boot: Boot) -> Result<Link, BootError> {
    let inbox = Arc::new(Inbox::new());
    let inbox_thread = Arc::clone(&inbox);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let surface_dropped = Arc::new(Ack::new());
    let dropped_thread = Arc::clone(&surface_dropped);
    let spawned = thread::Builder::new()
        .name("uifw-render".to_string())
        .spawn(move || {
            let result = render(boot, &inbox_thread, &ready_tx, &dropped_thread);
            let _ = done_tx.send(result);
        });
    if spawned.is_err() {
        return Err(BootError::Engine(engine::Status::NotReady));
    }
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Link {
            inbox,
            done: Mutex::new(Some(done_rx)),
            surface_dropped,
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(BootError::Engine(engine::Status::NotReady)),
    }
}

fn render(
    mut boot: Boot,
    inbox: &Arc<Inbox>,
    ready: &SyncSender<Result<(), BootError>>,
    surface_dropped: &Ack,
) -> Result<(), BootError> {
    let services = Rc::new(RefCell::new(boot.services));
    let engine = {
        let services = services.borrow();
        match prepare_engine(services.as_ref(), &boot.font_family) {
            Ok(engine) => engine,
            Err(error) => {
                let _ = ready.send(Err(error.clone()));
                return Err(error);
            }
        }
    };
    let log = {
        let services = Rc::clone(&services);
        Rc::new(move |level: LogLevel, message: &str| {
            services.borrow().log(level, message);
        }) as Rc<dyn Fn(LogLevel, &str)>
    };
    let mut session = None;
    let mut fixture = None;
    match boot.launch {
        Launch::Fixture(list) => fixture = Some(list),
        Launch::Session(open) => {
            let slot = EngineSlot {
                engine: Rc::clone(&engine),
                log: Rc::clone(&log),
            };
            match open(&slot) {
                Ok(opened) => session = Some(opened),
                Err(error) => {
                    let error = BootError::Session(error);
                    log(LogLevel::Error, &error.to_string());
                    let _ = ready.send(Err(error.clone()));
                    return Err(error);
                }
            }
        }
    }
    if ready.send(Ok(())).is_err() {
        return Ok(());
    }
    let inbox_frames = Arc::clone(inbox);
    boot.vsync.bind(Arc::new(move |time_ms| {
        inbox_frames.post(Command::Frame { time_ms });
    }));

    let mut input = Input::new();
    let mut surface: Option<Box<dyn Surface>> = None;
    let mut last_submitted: Option<DrawList> = None;
    let mut point_scale = 1.0_f32;
    let mut fixture_presented = false;
    let mut ime_shown = false;
    let mut needs_present = false;

    while let Some(command) = inbox.recv() {
        match command {
            Command::Shutdown => break,
            Command::SurfaceDestroyed => {
                apply_followup(
                    &mut surface,
                    &mut last_submitted,
                    &mut needs_present,
                    &mut fixture_presented,
                    surface_lost(),
                    false,
                    boot.vsync.as_ref(),
                    surface_dropped,
                );
            }
            Command::SurfaceReady(target) => {
                note_scale(
                    &mut last_submitted,
                    &mut point_scale,
                    target.metrics.device_pixel_ratio,
                );
                if input.set_metrics(target.metrics) {
                    boot.vsync.request_frame();
                }
                let opened = {
                    let mut engine = engine.borrow_mut();
                    (boot.open_surface)(&mut engine, target)
                };
                match opened {
                    Ok(opened) => {
                        surface = Some(opened);
                        boot.vsync.request_frame();
                    }
                    Err(engine::Status::NotReady) => {
                        surface = None;
                        last_submitted = None;
                        needs_present = false;
                        surface_dropped.signal();
                        boot.vsync.cancel();
                    }
                    Err(status) => {
                        let error = BootError::Engine(status);
                        log(LogLevel::Error, &error.to_string());
                        return Err(error);
                    }
                }
            }
            Command::SurfaceChanged(target) => {
                note_scale(
                    &mut last_submitted,
                    &mut point_scale,
                    target.metrics.device_pixel_ratio,
                );
                if input.set_metrics(target.metrics) {
                    boot.vsync.request_frame();
                }
            }
            Command::Pointer(event) => {
                if input.push_pointer(event) {
                    boot.vsync.request_frame();
                }
            }
            Command::Input(event) => {
                if input.push_input(event) {
                    boot.vsync.request_frame();
                }
            }
            Command::ImeDismissed => {
                if input.dismiss_ime() {
                    boot.vsync.request_frame();
                }
            }
            Command::Metrics(metrics) => {
                note_scale(
                    &mut last_submitted,
                    &mut point_scale,
                    metrics.device_pixel_ratio,
                );
                if input.set_metrics(metrics) {
                    boot.vsync.request_frame();
                }
            }
            Command::Lifecycle(Lifecycle::Pause) => {
                if let Some(session) = session.as_mut() {
                    session.lifecycle(Lifecycle::Pause);
                }
                if input.pause() {
                    boot.vsync.request_frame();
                }
            }
            Command::Lifecycle(Lifecycle::Resume) => {
                if let Some(session) = session.as_mut() {
                    session.lifecycle(Lifecycle::Resume);
                }
                if input.resume() {
                    boot.vsync.request_frame();
                }
            }
            Command::Frame { time_ms } => {
                if frame_step(&FrameView {
                    surface_open: surface.is_some(),
                    paused: false,
                    fixture_open: false,
                    fixture_presented: false,
                    unchanged: false,
                    needs_present: true,
                    again: false,
                }) == FrameStep::Cancel
                {
                    boot.vsync.cancel();
                    continue;
                }
                let frame = input.begin_frame();
                if let Some(metrics) = frame.metrics {
                    point_scale = metrics.device_pixel_ratio;
                    if let Some(session) = session.as_mut() {
                        session.metrics(&metrics);
                    }
                }
                if let Some(session) = session.as_mut() {
                    for event in frame.pointers {
                        session.pointer(event);
                    }
                    for event in frame.inputs {
                        session.input(event);
                    }
                    if frame.ime_dismissed {
                        session.ime_dismissed();
                    }
                }
                if let Some(session) = session.as_mut() {
                    publish_ime(session.as_mut(), &services, &mut ime_shown);
                }
                if frame_step(&FrameView {
                    surface_open: true,
                    paused: frame.paused,
                    fixture_open: fixture.is_some(),
                    fixture_presented,
                    unchanged: false,
                    needs_present: true,
                    again: false,
                }) == FrameStep::Cancel
                {
                    boot.vsync.cancel();
                    continue;
                }
                let mut again = false;
                let list = if let Some(session) = session.as_mut() {
                    let list = session.frame(time_ms);
                    again = session.wants_frame();
                    publish_ime(session.as_mut(), &services, &mut ime_shown);
                    list
                } else if let Some(list) = fixture.clone() {
                    list
                } else {
                    boot.vsync.cancel();
                    continue;
                };
                let unchanged = same_draw_list(last_submitted.as_ref(), &list);
                match frame_step(&FrameView {
                    surface_open: surface.is_some(),
                    paused: false,
                    fixture_open: false,
                    fixture_presented: false,
                    unchanged,
                    needs_present,
                    again,
                }) {
                    FrameStep::Cancel | FrameStep::Skip { again: false } => {
                        boot.vsync.cancel();
                        continue;
                    }
                    FrameStep::Skip { again: true } => {
                        boot.vsync.request_frame();
                        continue;
                    }
                    FrameStep::Submit => {
                        let services_for_image = services.borrow();
                        let submitted = engine.borrow_mut().submit(&list, point_scale, |source| {
                            services_for_image.image(source)
                        });
                        if let Err(status) = submitted {
                            log(LogLevel::Error, &format!("scene submit failed: {status}"));
                            boot.vsync.cancel();
                            continue;
                        }
                        last_submitted = Some(list);
                    }
                    FrameStep::PresentCached => {}
                }
                let presented = {
                    let Some(surface) = surface.as_mut() else {
                        boot.vsync.cancel();
                        continue;
                    };
                    surface.present(&mut engine.borrow_mut())
                };
                let after = after_present(presented, again);
                let followup = present_followup(after, fixture.is_some());
                let stop = apply_followup(
                    &mut surface,
                    &mut last_submitted,
                    &mut needs_present,
                    &mut fixture_presented,
                    followup,
                    matches!(after, AfterPresent::Shown { .. }),
                    boot.vsync.as_ref(),
                    surface_dropped,
                );
                if stop {
                    break;
                }
            }
        }
    }
    boot.vsync.cancel();
    Ok(())
}

fn open_engine() -> Result<engine::Engine, engine::Status> {
    #[cfg(target_os = "android")]
    {
        engine::Engine::create_vulkan()
    }
    #[cfg(not(target_os = "android"))]
    {
        engine::Engine::create_metal()
    }
}

fn prepare_engine(
    services: &dyn crate::Services,
    family: &str,
) -> Result<Rc<RefCell<engine::Engine>>, BootError> {
    let mut engine = match open_engine() {
        Ok(engine) => engine,
        Err(status) => {
            let error = BootError::Engine(status);
            services.log(LogLevel::Error, &error.to_string());
            return Err(error);
        }
    };
    let Some(font) = services.font(family) else {
        let error = BootError::Font(family.to_owned());
        services.log(LogLevel::Error, &error.to_string());
        return Err(error);
    };
    if let Err(status) = engine.register_font(font, family) {
        let error = BootError::Engine(status);
        services.log(LogLevel::Error, &error.to_string());
        return Err(error);
    }
    Ok(Rc::new(RefCell::new(engine)))
}

fn publish_ime(
    session: &mut dyn session::Session,
    services: &RefCell<Box<dyn crate::Services>>,
    shown: &mut bool,
) {
    let want = session.wants_ime();
    if want != *shown {
        services.borrow().set_ime(want);
        *shown = want;
    }
    if let Some(text) = session.take_ime_text() {
        if want {
            services.borrow().set_ime_text(&text);
        }
    }
}

fn apply_followup(
    surface: &mut Option<Box<dyn Surface>>,
    last_submitted: &mut Option<DrawList>,
    needs_present: &mut bool,
    fixture_presented: &mut bool,
    followup: PresentFollowup,
    write_fixture: bool,
    vsync: &dyn crate::Vsync,
    surface_dropped: &Ack,
) -> bool {
    *needs_present = followup.needs_present;
    if write_fixture {
        *fixture_presented = followup.mark_fixture_presented;
    }
    if followup.clear_surface {
        *surface = None;
        *last_submitted = None;
        surface_dropped.signal();
    }
    match followup.vsync {
        VsyncOp::Request => vsync.request_frame(),
        VsyncOp::Cancel => vsync.cancel(),
        VsyncOp::Stop => {
            vsync.cancel();
            return true;
        }
    }
    false
}

fn note_scale(last_submitted: &mut Option<DrawList>, point_scale: &mut f32, next: f32) {
    if (next - *point_scale).abs() > f32::EPSILON && last_submitted.is_some() {
        *last_submitted = None;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use session::{InputEvent, Session};

    use super::{
        AfterPresent, FrameStep, FrameView, VsyncOp, frame_step, present_followup, surface_lost,
    };

    /// A fixture frame has no session, so queued text is dropped.
    fn deliver_inputs(session: Option<&mut dyn Session>, inputs: VecDeque<InputEvent>) -> usize {
        let Some(session) = session else {
            return 0;
        };
        let mut count = 0;
        for event in inputs {
            session.input(event);
            count += 1;
        }
        count
    }

    fn view(overrides: FrameView) -> FrameView {
        overrides
    }

    fn open_view() -> FrameView {
        FrameView {
            surface_open: true,
            paused: false,
            fixture_open: false,
            fixture_presented: false,
            unchanged: false,
            needs_present: false,
            again: false,
        }
    }

    #[test]
    fn a_closed_surface_pauses_and_a_finished_fixture_cancel() {
        assert_eq!(
            frame_step(&view(FrameView {
                surface_open: false,
                ..open_view()
            })),
            FrameStep::Cancel
        );
        assert_eq!(
            frame_step(&view(FrameView {
                paused: true,
                ..open_view()
            })),
            FrameStep::Cancel
        );
        assert_eq!(
            frame_step(&view(FrameView {
                fixture_open: true,
                fixture_presented: true,
                ..open_view()
            })),
            FrameStep::Cancel
        );
    }

    #[test]
    fn an_unchanged_list_skips_unless_present_must_retry() {
        assert_eq!(
            frame_step(&view(FrameView {
                unchanged: true,
                again: true,
                ..open_view()
            })),
            FrameStep::Skip { again: true }
        );
        assert_eq!(
            frame_step(&view(FrameView {
                unchanged: true,
                ..open_view()
            })),
            FrameStep::Skip { again: false }
        );
        assert_eq!(
            frame_step(&view(FrameView {
                unchanged: true,
                needs_present: true,
                ..open_view()
            })),
            FrameStep::PresentCached
        );
        assert_eq!(frame_step(&open_view()), FrameStep::Submit);
    }

    #[test]
    fn a_failed_present_asks_for_another_frame_and_surface_loss_clears() {
        let retry = present_followup(AfterPresent::Retry, false);
        assert!(retry.needs_present);
        assert_eq!(retry.vsync, VsyncOp::Request);
        assert!(!retry.clear_surface);
        let lost = present_followup(AfterPresent::WaitForSurface, true);
        assert!(!lost.needs_present);
        assert!(lost.clear_surface);
        assert_eq!(lost.vsync, VsyncOp::Cancel);
        assert_eq!(surface_lost(), lost);
        let shown = present_followup(AfterPresent::Shown { again: false }, true);
        assert!(shown.mark_fixture_presented);
        assert_eq!(shown.vsync, VsyncOp::Cancel);
    }

    #[test]
    fn input_without_a_session_is_ignored() {
        let events = VecDeque::from([InputEvent::Text("a".to_string())]);
        assert_eq!(deliver_inputs(None, events), 0);
    }
}
