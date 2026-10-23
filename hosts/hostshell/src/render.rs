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
use crate::queue::Inbox;
use crate::shaper::ImpellerShaper;
use crate::{Boot, Launch, LogLevel, Presented, Surface};

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

    while let Some(command) = inbox.recv() {
        match command {
            Command::Shutdown => break,
            Command::SurfaceDestroyed => {
                surface = None;
                last_submitted = None;
                surface_dropped.signal();
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
                if surface.is_none() {
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
                if frame.paused || (fixture.is_some() && fixture_presented) {
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
                if same_draw_list(last_submitted.as_ref(), &list) {
                    if again {
                        boot.vsync.request_frame();
                    } else {
                        boot.vsync.cancel();
                    }
                    continue;
                }
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
                let presented = {
                    let Some(surface) = surface.as_mut() else {
                        boot.vsync.cancel();
                        continue;
                    };
                    surface.present(&mut engine.borrow_mut())
                };
                match presented {
                    Ok(Presented::Continue) => {
                        fixture_presented = fixture.is_some();
                        if again {
                            boot.vsync.request_frame();
                        } else {
                            boot.vsync.cancel();
                        }
                    }
                    Ok(Presented::Stop) => {
                        boot.vsync.cancel();
                        break;
                    }
                    Err(engine::Status::NotReady) => {
                        surface = None;
                        last_submitted = None;
                        surface_dropped.signal();
                        boot.vsync.cancel();
                    }
                    Err(status) => {
                        log(LogLevel::Error, &format!("draw failed: {status}"));
                        boot.vsync.cancel();
                    }
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

    #[test]
    fn input_without_a_session_is_ignored() {
        let events = VecDeque::from([InputEvent::Text("a".to_string())]);
        assert_eq!(deliver_inputs(None, events), 0);
    }
}
