//! Render thread. Commands come from the platform thread. This thread owns the
//! engine, the session, and the surface.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use session::FrameTime;

use crate::command::{Command, SurfaceId};
use crate::error::BootError;
use crate::images::{self, ImageCache, ImageDecoder, ImageError, ImageReady, ImageRequest};
use crate::input::Input;
use crate::present::{AfterPresent, after_present};
use crate::queue::Inbox;
use crate::scheduler::{self, FrameScheduler, FrameStep, FrameView, VsyncOp};
use crate::shaper::ImpellerShaper;
use crate::surface::{Changed, Destroyed, Ready, SurfaceHost};
use crate::timing::FrameTiming;
use crate::{Boot, Launch, LogLevel, Services, Surface};

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

    /// Delete one image cache key and arm a single frame so the next paint requests it again.
    pub fn reload_image(&self, source: &str) {
        self.inbox.post(Command::ReloadImage(source.to_owned()));
    }
}

struct Acks {
    pending: Mutex<HashMap<SurfaceId, bool>>,
    cv: Condvar,
}

impl Acks {
    fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            cv: Condvar::new(),
        }
    }

    fn arm(&self, id: SurfaceId) {
        lock(&self.pending).insert(id, false);
    }

    fn signal(&self, id: SurfaceId) {
        if let Some(done) = lock(&self.pending).get_mut(&id) {
            *done = true;
        }
        self.cv.notify_all();
    }

    fn signal_all(&self) {
        for done in lock(&self.pending).values_mut() {
            *done = true;
        }
        self.cv.notify_all();
    }

    fn poll(&self, id: SurfaceId) -> bool {
        let mut pending = lock(&self.pending);
        if pending.get(&id).copied().unwrap_or(false) {
            pending.remove(&id);
            true
        } else {
            false
        }
    }
}

pub struct Link {
    inbox: Arc<Inbox>,
    done: Mutex<Option<Receiver<Result<(), BootError>>>>,
    acks: Arc<Acks>,
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

    /// Delete one image cache key and arm a single frame so the next paint requests it again.
    pub fn reload_image(&self, source: &str) {
        self.poster().reload_image(source);
    }

    /// Posts destruction and remembers `id`. The platform thread keeps the window
    /// until [`Link::poll_dropped`] reports the render thread has dropped it.
    pub fn begin_destroy(&self, id: SurfaceId) {
        self.acks.arm(id);
        self.inbox.post(Command::SurfaceDestroyed { id });
    }

    pub fn poll_dropped(&self, id: SurfaceId) -> bool {
        self.acks.poll(id)
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
                Err(_) => Err(BootError::Panicked),
            },
            None => Ok(()),
        }
    }
}

struct ExitGuard {
    done: Option<Sender<Result<(), BootError>>>,
    acks: Arc<Acks>,
}

impl ExitGuard {
    fn finish(mut self, result: Result<(), BootError>) {
        if let Some(done) = self.done.take() {
            let _ = done.send(result);
        }
    }
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            let _ = done.send(Err(BootError::Panicked));
            self.acks.signal_all();
        }
    }
}

pub fn start(boot: Boot) -> Result<Link, BootError> {
    let inbox = Arc::new(Inbox::new());
    let inbox_thread = Arc::clone(&inbox);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::channel();
    let acks = Arc::new(Acks::new());
    let acks_thread = Arc::clone(&acks);
    let (image_tx, image_rx) = mpsc::channel();
    let decoder = Arc::clone(&boot.decoder);
    let inbox_images = Arc::clone(&inbox);
    let _io = thread::Builder::new()
        .name("uifw-io".to_string())
        .spawn(move || {
            decode_loop(decoder, image_rx, &inbox_images);
        });
    let (net_tx, net_rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    let inbox_net = Arc::clone(&inbox);
    let bridge = Arc::clone(&boot.bridge);
    let _net = crate::net::spawn(net_rx, inbox_net, bridge);
    let _time = crate::net::wake(wake_rx, Arc::clone(&inbox));
    let spawned = thread::Builder::new()
        .name("uifw-render".to_string())
        .spawn(move || {
            let guard = ExitGuard {
                done: Some(done_tx),
                acks: Arc::clone(&acks_thread),
            };
            let result = render(
                boot,
                &inbox_thread,
                &ready_tx,
                &acks_thread,
                image_tx,
                net_tx,
                wake_tx,
            );
            guard.finish(result);
        });
    if spawned.is_err() {
        return Err(BootError::Engine(engine::Status::NotReady));
    }
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Link {
            inbox,
            done: Mutex::new(Some(done_rx)),
            acks,
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(BootError::Engine(engine::Status::NotReady)),
    }
}

fn decode_loop(decoder: Arc<dyn ImageDecoder>, jobs: Receiver<ImageRequest>, inbox: &Inbox) {
    while let Ok(request) = jobs.recv() {
        inbox.post(Command::ImageReady(decode_one(decoder.as_ref(), &request)));
    }
}

fn decode_one(decoder: &dyn ImageDecoder, request: &ImageRequest) -> ImageReady {
    let result = if request.source.starts_with("https://") {
        match crate::net::fetch_image_body(&request.source) {
            Ok(bytes) => decoder.decode_bytes(&request.source, &bytes),
            Err(error) => Err(error),
        }
    } else if request.source.starts_with("http://") {
        Err(ImageError::Failed("image URL must be https".into()))
    } else {
        decoder.decode_at(&request.source, request.scale_bucket)
    };
    match result {
        Ok(mut handle) => {
            if !images::pixel_count_ok(handle.size) {
                return ImageReady::Failed {
                    source: Arc::clone(&request.source),
                    generation: request.generation,
                    message: "pixel limit".into(),
                };
            }
            handle.source = Arc::clone(&request.source);
            handle.generation = request.generation;
            ImageReady::Decoded(handle)
        }
        Err(ImageError::Missing) => ImageReady::Missing {
            source: Arc::clone(&request.source),
            generation: request.generation,
        },
        Err(ImageError::Failed(message)) => ImageReady::Failed {
            source: Arc::clone(&request.source),
            generation: request.generation,
            message,
        },
    }
}

struct Installed {
    list: protocol::DrawList,
    scale: f32,
}

struct Runtime {
    services: Rc<RefCell<Box<dyn Services>>>,
    engine: Rc<RefCell<engine::Engine>>,
    log: Rc<dyn Fn(LogLevel, &str)>,
    session: Option<Box<dyn session::Session>>,
    fixture: Option<protocol::DrawList>,
    input: Input,
    surfaces: SurfaceHost,
    scheduler: FrameScheduler,
    vsync: Arc<dyn crate::Vsync>,
    open_surface: Box<
        dyn FnMut(
                &mut engine::Engine,
                crate::command::SurfaceTarget,
            ) -> Result<Box<dyn Surface>, engine::Status>
            + Send,
    >,
    installed: Option<Installed>,
    point_scale: f32,
    fixture_presented: bool,
    needs_present: bool,
    images: ImageCache,
    image_tx: Sender<ImageRequest>,
    frame_number: u64,
    semantics: Option<ui_core::SemanticsUpdate>,
    wake_tx: Sender<Option<i64>>,
}

fn render(
    boot: Boot,
    inbox: &Arc<Inbox>,
    ready: &SyncSender<Result<(), BootError>>,
    acks: &Acks,
    image_tx: Sender<ImageRequest>,
    net_tx: Sender<session::IoJob>,
    wake_tx: Sender<Option<i64>>,
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
                Ok(mut opened) => {
                    let submit = net_tx.clone();
                    opened.bind_io(Box::new(move |job| {
                        let _ = submit.send(job);
                    }));
                    session = Some(opened);
                }
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
    boot.vsync.bind(Arc::new(move |token, mut time| {
        time.mono_ms = crate::net::mono_ms();
        inbox_frames.post(Command::Frame { token, time });
    }));
    let mut runtime = Runtime {
        services,
        engine,
        log,
        session,
        fixture,
        input: Input::new(),
        surfaces: SurfaceHost::new(),
        scheduler: FrameScheduler::new(),
        vsync: boot.vsync,
        open_surface: boot.open_surface,
        installed: None,
        point_scale: 1.0,
        fixture_presented: false,
        needs_present: false,
        images: ImageCache::new(),
        image_tx,
        frame_number: 0,
        semantics: None,
        wake_tx,
    };
    while let Some(command) = inbox.recv() {
        if on_command(&mut runtime, command, acks)? {
            break;
        }
    }
    runtime.vsync.cancel();
    Ok(())
}

fn on_command(runtime: &mut Runtime, command: Command, acks: &Acks) -> Result<bool, BootError> {
    match command {
        Command::Shutdown => Ok(true),
        Command::SurfaceDestroyed { id } => {
            if runtime.surfaces.destroyed(id) == Destroyed::Dropped {
                let follow = scheduler::surface_lost();
                runtime.installed = None;
                runtime.needs_present = follow.needs_present;
                runtime.scheduler.cancel();
                runtime.vsync.cancel();
            }
            acks.signal(id);
            Ok(false)
        }
        Command::SurfaceReady(target) => {
            note_scale(runtime, target.metrics.device_pixel_ratio);
            let metrics = target.metrics;
            let opened = {
                let mut engine = runtime.engine.borrow_mut();
                runtime
                    .surfaces
                    .ready(target, &mut engine, runtime.open_surface.as_mut())
            };
            match opened {
                Ready::Opened { id, replaced } => {
                    let _ = id;
                    if let Some(old) = replaced {
                        acks.signal(old);
                    }
                    if runtime.input.set_metrics(metrics) {
                        arm(runtime);
                    }
                    arm(runtime);
                }
                Ready::Failed(engine::Status::NotReady) => {
                    runtime.installed = None;
                    runtime.needs_present = false;
                    runtime.scheduler.cancel();
                    runtime.vsync.cancel();
                }
                Ready::Failed(status) => {
                    let error = BootError::Engine(status);
                    (runtime.log)(LogLevel::Error, &error.to_string());
                    return Err(error);
                }
            }
            Ok(false)
        }
        Command::SurfaceChanged { id, metrics } => {
            if runtime.surfaces.changed(id, metrics) == Changed::Staged {
                note_scale(runtime, metrics.device_pixel_ratio);
                if runtime.input.set_metrics(metrics) {
                    arm(runtime);
                }
            }
            Ok(false)
        }
        Command::Pointer(event) => {
            if runtime.input.push_pointer(event) {
                arm(runtime);
            }
            Ok(false)
        }
        Command::Input(event) => {
            if runtime.input.push_input(event) {
                arm(runtime);
            }
            Ok(false)
        }
        Command::ImeDismissed => {
            if runtime.input.dismiss_ime() {
                arm(runtime);
            }
            Ok(false)
        }
        Command::Metrics(metrics) => {
            note_scale(runtime, metrics.device_pixel_ratio);
            if runtime.input.set_metrics(metrics) {
                arm(runtime);
            }
            Ok(false)
        }
        Command::Lifecycle(state) => {
            let deadline = runtime.session.as_mut().map(|session| {
                session.lifecycle(state);
                session.next_wake_ms().map(|ms| ms as i64)
            });
            if let Some(deadline) = deadline {
                note_wake(runtime, deadline);
            }
            let arm_frame = match state {
                session::Lifecycle::Hidden | session::Lifecycle::Paused => runtime.input.pause(),
                session::Lifecycle::Resumed => runtime.input.resume(),
                session::Lifecycle::Inactive | session::Lifecycle::Detached => false,
            };
            if arm_frame {
                arm(runtime);
            }
            Ok(false)
        }
        Command::Settings(settings) => {
            if let Some(session) = runtime.session.as_mut() {
                session.settings(&settings);
                if session.wants_frame() {
                    arm(runtime);
                }
            }
            Ok(false)
        }
        Command::WindowFocus(focused) => {
            if let Some(session) = runtime.session.as_mut() {
                session.window_focus(focused);
            }
            Ok(false)
        }
        Command::SemanticsAction(action) => {
            if let Some(session) = runtime.session.as_mut() {
                session.semantics_action(action);
                arm(runtime);
            }
            Ok(false)
        }
        Command::ImageReady(ready) => {
            if let Some(message) = runtime.images.apply(ready) {
                (runtime.log)(LogLevel::Error, &message);
            }
            arm(runtime);
            Ok(false)
        }
        Command::ReloadImage(source) => {
            runtime.images.reload(&source);
            arm(runtime);
            Ok(false)
        }
        Command::HttpDone { token, result } => {
            if let Some(session) = runtime.session.as_mut() {
                session.http_result(token, result);
                if session.wants_frame() {
                    arm(runtime);
                }
            }
            Ok(false)
        }
        Command::PluginDone { token, result } => {
            if let Some(session) = runtime.session.as_mut() {
                session.plugin_result(token, result);
                if session.wants_frame() {
                    arm(runtime);
                }
            }
            Ok(false)
        }
        Command::Socket {
            id,
            event,
            text,
            data,
        } => {
            if let Some(session) = runtime.session.as_mut() {
                session.socket_event(id, &event, text, &data);
                if session.wants_frame() {
                    arm(runtime);
                }
            }
            Ok(false)
        }
        Command::Wake => {
            if runtime.input.is_paused() || runtime.scheduler.busy() {
                return Ok(false);
            }
            let now = crate::net::mono_ms();
            on_frame(
                runtime,
                FrameTime {
                    vsync_ms: now,
                    target_ms: now,
                    interval_ms: 16.0,
                    mono_ms: now,
                },
            )
        }
        Command::Frame { token, time } => {
            if !runtime.scheduler.accept(token) {
                return Ok(false);
            }
            on_frame(runtime, time)
        }
    }
}

fn on_frame(runtime: &mut Runtime, time: FrameTime) -> Result<bool, BootError> {
    if !runtime.surfaces.is_open() {
        return Ok(end_frame(runtime, false, false));
    }
    let frame = runtime.input.begin_frame();
    if let Some(metrics) = frame.metrics {
        runtime.point_scale = metrics.device_pixel_ratio;
        if let Some(session) = runtime.session.as_mut() {
            session.metrics(&metrics);
        }
    }
    let mut leave = false;
    if let Some(session) = runtime.session.as_mut() {
        for event in frame.pointers {
            session.pointer(event);
        }
        for event in frame.inputs {
            session.input(event);
        }
        if let Some(text) = session.take_clipboard() {
            runtime.services.borrow().set_clipboard(&text);
        }
        leave = session.take_exit();
        if frame.ime_dismissed {
            session.ime_dismissed();
        }
        crate::ime::publish(session.as_mut(), &runtime.services);
    }
    if leave {
        runtime.services.borrow().request_exit();
    }
    if frame.paused
        || scheduler::frame_step(&FrameView {
            surface_open: true,
            paused: frame.paused,
            fixture_open: runtime.fixture.is_some(),
            fixture_presented: runtime.fixture_presented,
            unchanged: false,
            needs_present: true,
            again: false,
        }) == FrameStep::Cancel
    {
        return Ok(end_frame(runtime, false, false));
    }
    runtime.frame_number = runtime.frame_number.wrapping_add(1);
    let session_started = Instant::now();
    let mut again = false;
    let list = if let Some(session) = runtime.session.as_mut() {
        let framed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let list = session.frame(time);
            let again = session.wants_frame();
            let semantics = session.semantics();
            (list, again, semantics)
        }));
        match framed {
            Ok((list, wants_another, update)) => {
                again = wants_another;
                if let Some(update) = update {
                    if runtime.semantics.as_ref() != Some(&update) {
                        runtime.services.borrow().semantics(&update);
                        runtime.semantics = Some(update);
                    }
                }
                crate::ime::publish(session.as_mut(), &runtime.services);
                list
            }
            Err(_) => {
                (runtime.log)(
                    LogLevel::Error,
                    "render thread panicked; keeping the previous frame",
                );
                again = false;
                runtime
                    .installed
                    .as_ref()
                    .map(|installed| installed.list.clone())
                    .unwrap_or(protocol::DrawList {
                        commands: Vec::new(),
                    })
            }
        }
    } else if let Some(list) = runtime.fixture.clone() {
        list
    } else {
        return Ok(end_frame(runtime, false, false));
    };
    let session_ms = elapsed_ms(session_started);
    let reloads = runtime
        .session
        .as_mut()
        .map(|session| session.take_reloads())
        .unwrap_or_default();
    let reloaded = !reloads.is_empty();
    for source in &reloads {
        runtime.images.reload(source);
    }
    again |= reloaded;
    if !images_ready(runtime, &list) {
        return Ok(end_frame(runtime, reloaded, false));
    }
    let unchanged = crate::input::same_draw_list(
        runtime.installed.as_ref().map(|installed| &installed.list),
        &list,
    ) && runtime
        .installed
        .as_ref()
        .is_some_and(|installed| installed.scale == runtime.point_scale);
    let mut submit_ms = 0.0;
    match scheduler::frame_step(&FrameView {
        surface_open: runtime.surfaces.is_open(),
        paused: false,
        fixture_open: false,
        fixture_presented: false,
        unchanged,
        needs_present: runtime.needs_present,
        again,
    }) {
        FrameStep::Cancel | FrameStep::Skip { again: false } => {
            return Ok(end_frame(runtime, false, false));
        }
        FrameStep::Skip { again: true } => {
            return Ok(end_frame(runtime, true, false));
        }
        FrameStep::Submit => {
            let fonts = runtime
                .session
                .as_mut()
                .map(|session| session.take_fonts())
                .unwrap_or_default();
            install_fonts(
                &mut runtime.engine.borrow_mut(),
                fonts,
                runtime.log.as_ref(),
            );
            let started = Instant::now();
            let pixels = runtime
                .images
                .snapshot(&list, images::scale_bucket(runtime.point_scale));
            let submitted =
                runtime
                    .engine
                    .borrow_mut()
                    .submit(&list, runtime.point_scale, |source| {
                        pixels.iter().find(|(name, ..)| name == source).map(
                            |(_, width, height, rgba)| engine::ImagePixels {
                                width: *width,
                                height: *height,
                                rgba,
                            },
                        )
                    });
            submit_ms = elapsed_ms(started);
            if let Err(status) = submitted {
                (runtime.log)(LogLevel::Error, &format!("scene submit failed: {status}"));
                if status == engine::Status::NotLinked {
                    return Err(BootError::Engine(status));
                }
                return Ok(end_frame(runtime, true, false));
            }
            runtime.installed = Some(Installed {
                list,
                scale: runtime.point_scale,
            });
        }
        FrameStep::PresentCached => {}
    }
    let started = Instant::now();
    let presented = {
        let mut engine = runtime.engine.borrow_mut();
        runtime.surfaces.present(&mut engine)
    };
    let present_ms = elapsed_ms(started);
    let timing = FrameTiming {
        number: runtime.frame_number,
        vsync_ms: time.vsync_ms,
        target_ms: time.target_ms,
        session_ms,
        submit_ms,
        present_ms,
    };
    if timing.over_budget(time.interval_ms) {
        (runtime.log)(
            LogLevel::Info,
            &format!(
                "frame {} session {:.2}ms submit {:.2}ms present {:.2}ms",
                timing.number, timing.session_ms, timing.submit_ms, timing.present_ms
            ),
        );
    }
    let after = after_present(presented, again);
    if after == AfterPresent::Fatal {
        return Err(BootError::Engine(engine::Status::NotLinked));
    }
    let follow = scheduler::present_followup(after, runtime.fixture.is_some());
    runtime.needs_present = follow.needs_present;
    if matches!(after, AfterPresent::Shown { .. }) {
        runtime.fixture_presented = follow.mark_fixture_presented;
    }
    if follow.clear_surface {
        runtime.surfaces.clear();
        runtime.installed = None;
    }
    let stop = follow.vsync == VsyncOp::Stop;
    let again = follow.vsync == VsyncOp::Request;
    Ok(end_frame(runtime, again, stop))
}

fn images_ready(runtime: &mut Runtime, list: &protocol::DrawList) -> bool {
    runtime.images.begin_frame();
    if let Some(session) = runtime.session.as_mut() {
        if let Some(counts) = session.image_source_counts() {
            runtime.images.sync_counts(&counts);
        }
    }
    let bucket = images::scale_bucket(runtime.point_scale);
    runtime.images.pin(list, bucket);
    let mut waiting = false;
    let mut blocked = false;
    for (source, required) in list.image_requirements() {
        if runtime.images.is_pending(&source, bucket) {
            if required {
                waiting = true;
            }
            continue;
        }
        if runtime.images.resolve(&source, bucket).is_some() {
            continue;
        }
        if runtime.images.is_negative(&source, bucket) {
            if required {
                blocked = true;
                if let Some(message) = runtime.images.take_required_log(&source, bucket) {
                    (runtime.log)(LogLevel::Error, &message);
                }
            }
            continue;
        }
        if let Some(request) = runtime.images.request(&source, bucket) {
            if runtime.image_tx.send(request).is_err() {
                (runtime.log)(LogLevel::Error, "image decoder stopped");
            }
        }
        if required {
            waiting = true;
        }
    }
    if blocked {
        return false;
    }
    !waiting
}

fn end_frame(runtime: &mut Runtime, again: bool, stop: bool) -> bool {
    if stop {
        runtime.scheduler.cancel();
        runtime.vsync.cancel();
        note_wake(runtime, None);
        return true;
    }
    let deadline = runtime.session.as_mut().and_then(|session| {
        if runtime.input.is_paused() {
            None
        } else {
            session.next_wake_ms().map(|ms| ms as i64)
        }
    });
    note_wake(runtime, deadline);
    if let Some(token) = runtime.scheduler.finish(again) {
        runtime.vsync.request_frame(token);
    } else {
        runtime.vsync.cancel();
    }
    false
}

fn note_wake(runtime: &Runtime, deadline: Option<i64>) {
    let _ = runtime.wake_tx.send(deadline);
}

fn arm(runtime: &mut Runtime) {
    if let Some(token) = runtime.scheduler.request() {
        runtime.vsync.request_frame(token);
    }
}

fn note_scale(runtime: &mut Runtime, next: f32) {
    if (next - runtime.point_scale).abs() > f32::EPSILON && runtime.installed.is_some() {
        runtime.installed = None;
    }
    runtime.point_scale = next;
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
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

/// Registers queued faces before `submit`. A failure logs the family and status
/// only, then the frame continues with the faces already on the engine.
fn install_fonts(
    engine: &mut engine::Engine,
    fonts: Vec<(String, Vec<u8>)>,
    log: &dyn Fn(LogLevel, &str),
) {
    for (family, bytes) in fonts {
        if let Err(status) = engine.register_font(&bytes, &family) {
            log(LogLevel::Error, &format!("font {family}: {status}"));
        }
    }
}

fn prepare_engine(
    services: &dyn Services,
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
    for (name, bytes) in services.extra_fonts() {
        if let Err(status) = engine.register_font(&bytes, &name) {
            services.log(LogLevel::Error, &format!("font {name}: {status}"));
        }
    }
    Ok(Rc::new(RefCell::new(engine)))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::install_fonts;
    use crate::LogLevel;

    #[test]
    fn a_rejected_font_is_logged_without_its_bytes() {
        let secret = b"SECRET-FONT-BYTES-DO-NOT-LOG";
        let lines = Mutex::new(Vec::new());
        let log = |level: LogLevel, message: &str| {
            lines
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push((level, message.to_owned()));
        };
        match engine::Engine::create_metal() {
            Ok(mut engine) => {
                install_fonts(
                    &mut engine,
                    vec![("Custom".to_owned(), secret.to_vec())],
                    &log,
                );
            }
            Err(engine::Status::NotLinked) => {
                log(
                    LogLevel::Error,
                    &format!("font Custom: {}", engine::Status::NotLinked),
                );
            }
            Err(status) => panic!("{status}"),
        }
        let lines = lines.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!lines.is_empty(), "a bad face must be reported");
        let message = &lines[0].1;
        assert!(message.contains("Custom"), "{message}");
        assert!(message.contains("font "), "{message}");
        assert!(!message.contains("SECRET-FONT-BYTES"), "{message}");
    }
}
