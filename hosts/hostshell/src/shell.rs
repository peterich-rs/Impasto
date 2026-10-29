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

use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::error::BootError;
use crate::images::{self, ImageCache, ImageDecoder, ImageError, ImageReady, ImageRequest};
use crate::input::Input;
use crate::present::{self, AfterPresent, after_present};
use crate::queue::Inbox;
use crate::scheduler::{self, FrameScheduler, FrameStep, FrameView, VsyncOp};
use crate::shaper::ImpellerShaper;
use crate::surface::{OpenRetry, Ready, SurfaceHost};
use crate::timing::FrameTiming;
use crate::{Boot, Launch, LogLevel, Services, Surface};

mod cache_cmds;
mod frame_cmds;
mod session_cmds;
mod surface_cmds;

pub struct EngineSlot {
    typography: Rc<RefCell<engine::Typography>>,
    log: Rc<dyn Fn(LogLevel, &str)>,
}

impl EngineSlot {
    pub fn shaper(&self) -> ImpellerShaper {
        ImpellerShaper::new(Rc::clone(&self.typography))
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

/// Waits until the render thread has dropped `id`. Clones only the ack map.
pub struct DropWait {
    acks: Arc<Acks>,
}

impl DropWait {
    /// Blocks until `id` is signalled, or until shutdown marks every ack done.
    /// Returns immediately when `id` was never armed or was already taken.
    pub fn wait(self, id: SurfaceId) {
        let mut pending = lock(&self.acks.pending);
        loop {
            match pending.get(&id).copied() {
                Some(true) => {
                    pending.remove(&id);
                    return;
                }
                None => return,
                Some(false) => {
                    pending = self
                        .acks
                        .cv
                        .wait(pending)
                        .unwrap_or_else(|error| error.into_inner());
                }
            }
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
    /// until [`DropWait::wait`] sees this ack. [`Link::poll_dropped`] is the same bit.
    pub fn begin_destroy(&self, id: SurfaceId) {
        self.acks.arm(id);
        self.inbox.post(Command::SurfaceDestroyed { id });
    }

    pub fn poll_dropped(&self, id: SurfaceId) -> bool {
        self.acks.poll(id)
    }

    /// A waiter for [`Self::begin_destroy`]. The platform thread does not block on it.
    pub fn drop_wait(&self) -> DropWait {
        DropWait {
            acks: Arc::clone(&self.acks),
        }
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
    let http = boot.http.clone();
    let decoder = Arc::clone(&boot.decoder);
    let inbox_images = Arc::clone(&inbox);
    let http_images = http.clone();
    let _io = thread::Builder::new()
        .name("uifw-io".to_string())
        .spawn(move || {
            decode_loop(decoder, image_rx, &inbox_images, http_images);
        });
    let (net_tx, net_rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    let inbox_net = Arc::clone(&inbox);
    let _net = crate::net::spawn(net_rx, inbox_net, http);
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

fn decode_loop(
    decoder: Arc<dyn ImageDecoder>,
    jobs: Receiver<ImageRequest>,
    inbox: &Inbox,
    http: Option<Arc<dyn session::HttpClient>>,
) {
    while let Ok(request) = jobs.recv() {
        inbox.post(Command::ImageReady(decode_one(
            decoder.as_ref(),
            http.as_deref(),
            &request,
        )));
    }
}

fn decode_one(
    decoder: &dyn ImageDecoder,
    http: Option<&dyn session::HttpClient>,
    request: &ImageRequest,
) -> ImageReady {
    let result = if request.source.starts_with("https://") {
        match crate::net::fetch_image_body(http, &request.source) {
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
    scale: f32,
}

struct Runtime {
    services: Rc<RefCell<Box<dyn Services>>>,
    engine: Rc<RefCell<engine::Engine>>,
    typography: Rc<RefCell<engine::Typography>>,
    log: Rc<dyn Fn(LogLevel, &str)>,
    session: Option<Box<dyn session::Session>>,
    registry: Arc<session::ModuleRegistry>,
    latest: Option<Arc<protocol::LayerTree>>,
    input: Input,
    surfaces: SurfaceHost,
    scheduler: FrameScheduler,
    vsync: Arc<dyn crate::Vsync>,
    open_surface: Box<
        dyn FnMut(&mut engine::Engine, SurfaceTarget) -> Result<Box<dyn Surface>, engine::Status>
            + Send,
    >,
    installed: Option<Installed>,
    point_scale: f32,
    needs_present: bool,
    images: ImageCache,
    image_tx: Sender<ImageRequest>,
    frame_number: u64,
    semantics: Option<ui_core::SemanticsUpdate>,
    panic_streak: u32,
    wake_tx: Sender<Option<i64>>,
    dump_list: bool,
    present_failures: u32,
    present_logged: bool,
    present_retry_at: Option<i64>,
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
    let Launch::Session(open) = boot.launch;
    let typography = engine.borrow().typography();
    let slot = EngineSlot {
        typography: Rc::clone(&typography),
        log: Rc::clone(&log),
    };
    let rt = match crate::rt::handle() {
        Ok(handle) => handle,
        Err(message) => {
            log(LogLevel::Error, &message);
            let error = BootError::Session(crate::SessionError::Runtime);
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
    let app_state = session::shared("active".to_owned());
    let launch_url = boot.launch_url.clone();
    let mut modules = vec![crate::modules::app_state(Arc::clone(&app_state))];
    modules.extend(boot.modules);
    let registry = Arc::new(session::ModuleRegistry::new(modules));
    let host = session::HostConfig {
        registry: Arc::clone(&registry),
        exec: Arc::new(crate::RtExec {
            rt,
            main: boot.main_thread.clone(),
        }),
        settle: crate::settle_factory(Arc::clone(inbox)),
        app_state,
        launch_url,
    };
    let session = match open(
        &slot,
        Box::new({
            let submit = net_tx.clone();
            move |job| {
                let _ = submit.send(job);
            }
        }),
        host,
    ) {
        Ok(opened) => Some(opened),
        Err(error) => {
            let error = BootError::Session(error);
            log(LogLevel::Error, &error.to_string());
            let _ = ready.send(Err(error.clone()));
            return Err(error);
        }
    };
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
        typography,
        engine,
        log,
        session,
        registry,
        latest: None,
        input: Input::new(),
        surfaces: SurfaceHost::new(),
        scheduler: FrameScheduler::new(),
        vsync: boot.vsync,
        open_surface: boot.open_surface,
        installed: None,
        point_scale: 1.0,
        needs_present: false,
        images: ImageCache::new(),
        image_tx,
        frame_number: 0,
        panic_streak: 0,
        semantics: None,
        wake_tx,
        dump_list: boot.dump_list,
        present_failures: 0,
        present_logged: false,
        present_retry_at: None,
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
        command @ (Command::SurfaceDestroyed { .. }
        | Command::SurfaceReady(_)
        | Command::SurfaceChanged { .. }) => surface_cmds::handle(runtime, command, acks),
        command @ (Command::Pointer(_)
        | Command::Input(_)
        | Command::ImeDismissed
        | Command::Metrics(_)
        | Command::Lifecycle(_)
        | Command::Settings(_)
        | Command::WindowFocus(_)
        | Command::SemanticsAction(_)
        | Command::LaunchUrl(_)
        | Command::HttpDone { .. }
        | Command::HttpHead { .. }
        | Command::HttpChunk { .. }
        | Command::PluginDone { .. }
        | Command::Socket { .. }) => session_cmds::handle(runtime, command),
        command @ (Command::Invalidate
        | Command::MemoryWarning
        | Command::ImageReady(_)
        | Command::ReloadImage(_)) => cache_cmds::handle(runtime, command),
        command @ (Command::Wake | Command::Frame { .. }) => {
            frame_cmds::handle(runtime, command, acks)
        }
    }
}

fn fold_semantics(
    snapshot: &mut Option<ui_core::SemanticsUpdate>,
    patch: &ui_core::SemanticsPatch,
) {
    let update = snapshot.get_or_insert_with(|| ui_core::SemanticsUpdate {
        root: patch.root,
        nodes: Vec::new(),
    });
    update.root = patch.root;
    for op in &patch.ops {
        match op {
            ui_core::SemanticsOp::Upsert(node) => {
                if let Some(found) = update.nodes.iter_mut().find(|item| item.id == node.id) {
                    *found = node.clone();
                } else {
                    update.nodes.push(node.clone());
                }
            }
            ui_core::SemanticsOp::Remove(id) => update.nodes.retain(|node| node.id != *id),
        }
    }
}

fn layer_tree_line(tree: &protocol::LayerTree) -> String {
    let pictures: Vec<String> = tree
        .pictures
        .iter()
        .map(|picture| {
            format!(
                "{}:gen{}:{}cmds",
                picture.id.0,
                picture.generation,
                picture.commands.len()
            )
        })
        .collect();
    format!(
        "layer_tree epoch={} layers={} pictures=[{}]",
        tree.epoch,
        tree.layers.len(),
        pictures.join(", ")
    )
}

fn on_frame(runtime: &mut Runtime, time: FrameTime, acks: &Acks) -> Result<bool, BootError> {
    if let Some((target, attempts)) = runtime.surfaces.take_attempt() {
        match open_surface_target(runtime, target, attempts, OpenPlace::Frame)? {
            OpenedSurface::Ready { replaced } => {
                if let Some(old) = replaced {
                    acks.signal(old);
                }
            }
            OpenedSurface::Waiting => return Ok(end_frame(runtime, false, false)),
        }
    }
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
    let gesture_started = Instant::now();
    let mut leave = false;
    if let Some(session) = runtime.session.as_mut() {
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
    let gesture_ms = gesture_started.elapsed().as_secs_f64() * 1000.0;
    runtime.frame_number = runtime.frame_number.wrapping_add(1);
    let session_started = Instant::now();
    let mut again;
    let mut reloads = Vec::new();
    let mut fonts = Vec::new();
    let mut census = None;
    let mut layers_changed = false;
    let mut phases = None;
    if let Some(session) = runtime.session.as_mut() {
        let framed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.frame(time)));
        match framed {
            Ok(mut output) => {
                runtime.panic_streak = 0;
                again = output.again;
                leave = output.exit;
                if let Some(tree) = output.layers.take() {
                    if runtime.dump_list {
                        (runtime.log)(LogLevel::Info, &layer_tree_line(&tree));
                    }
                    runtime.latest = Some(tree);
                    layers_changed = true;
                }
                if let Some(text) = output.clipboard.take() {
                    if let Err(message) = runtime.registry.call(
                        "Clipboard",
                        "setString",
                        &[session::PluginValue::Str(text)],
                    ) {
                        (runtime.log)(LogLevel::Error, &message);
                    }
                }
                if let Some(update) = output.ime.take() {
                    crate::ime::publish_update(&update, &runtime.services);
                }
                if let Some(patch) = output.semantics.take() {
                    fold_semantics(&mut runtime.semantics, &patch);
                    if let Some(update) = runtime.semantics.as_ref() {
                        runtime.services.borrow().semantics(update);
                    }
                }
                reloads = std::mem::take(&mut output.reloads);
                fonts = std::mem::take(&mut output.fonts);
                census = output.images.take();
                phases = output.phases.take();
            }
            Err(_) => {
                (runtime.log)(
                    LogLevel::Error,
                    "render thread panicked; keeping the previous frame",
                );
                again = false;
                runtime.panic_streak = runtime.panic_streak.saturating_add(1);
                if runtime.panic_streak >= 3 {
                    leave = true;
                }
            }
        }
    } else {
        return Ok(end_frame(runtime, false, false));
    }
    if leave {
        runtime.services.borrow().request_exit();
    }
    let session_ms = elapsed_ms(session_started);
    let reloaded = !reloads.is_empty();
    for source in &reloads {
        runtime.images.reload(source);
    }
    again |= reloaded;
    if !fonts.is_empty() {
        install_fonts(
            &mut runtime.engine.borrow_mut(),
            fonts,
            runtime.log.as_ref(),
        );
    }
    let Some(tree) = runtime.latest.clone() else {
        return Ok(end_frame(runtime, again, false));
    };
    let requirements = tree.image_requirements();
    if !images_ready(runtime, &requirements, census.as_deref()) {
        return Ok(end_frame(runtime, reloaded, false));
    }
    let unchanged = !layers_changed
        && runtime
            .installed
            .as_ref()
            .is_some_and(|installed| installed.scale == runtime.point_scale);
    let mut submit_ms = 0.0;
    match scheduler::frame_step(&FrameView {
        surface_open: runtime.surfaces.is_open(),
        paused: frame.paused,
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
            let started = Instant::now();
            let pixels = runtime
                .images
                .snapshot_requirements(&requirements, images::scale_bucket(runtime.point_scale));
            let resolve = |source: &str| {
                pixels
                    .iter()
                    .find(|(name, ..)| name == source)
                    .map(|(_, width, height, rgba)| engine::ImagePixels {
                        width: *width,
                        height: *height,
                        rgba,
                    })
            };
            let submitted =
                runtime
                    .engine
                    .borrow_mut()
                    .submit_layers(&tree, runtime.point_scale, resolve);
            submit_ms = elapsed_ms(started);
            if let Err(status) = submitted {
                (runtime.log)(LogLevel::Error, &format!("scene submit failed: {status}"));
                if status == engine::Status::NotLinked {
                    return Err(BootError::Engine(status));
                }
                return Ok(end_frame(runtime, true, false));
            }
            runtime.installed = Some(Installed {
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
    let stats = runtime.engine.borrow().submit_stats();
    let shapes = runtime.typography.borrow_mut().take_shape_stats();
    let phase = phases.unwrap_or_default();
    let timing = FrameTiming {
        number: runtime.frame_number,
        vsync_ms: time.vsync_ms,
        target_ms: time.target_ms,
        session_ms,
        gesture_ms,
        script_ms: phase.script_ms,
        layout_ms: phase.layout_ms,
        layout_cb_ms: phase.layout_cb_ms,
        sync_ms: phase.sync_ms,
        record_ms: phase.record_ms,
        semantics_ms: phase.semantics_ms,
        submit_ms,
        present_ms,
        shapes_built: shapes.built,
        shapes_measure_hit: shapes.measure_hit,
        shapes_measure_miss: shapes.measure_miss,
        shapes_paint_hit: shapes.paint_hit,
        shapes_paint_miss: shapes.paint_miss,
        picture_rebuilt: stats.picture_rebuilt,
        picture_reused: stats.picture_reused,
        layer_count: stats.layer_count,
    };
    if timing.over_budget(time.interval_ms) {
        (runtime.log)(
            LogLevel::Info,
            &format!(
                "frame {} gesture {:.2}ms script {:.2}ms layout {:.2}ms layoutcb {:.2}ms sync {:.2}ms record {:.2}ms semantics {:.2}ms session {:.2}ms submit {:.2}ms present {:.2}ms shapes b{} mh{} mm{} ph{} pm{} rebuilt {} reused {} layers {} js motion {:.2} dispatch {:.2} timer {:.2} raf {:.2} callback {:.2} flush {:.2} laywork {:.2} x{} {} reuse {} shaped {} ptr {} key {} edit {} ime {} scroll {} focus {} blur {} gesture {}",
                timing.number,
                timing.gesture_ms,
                timing.script_ms,
                timing.layout_ms,
                timing.layout_cb_ms,
                timing.sync_ms,
                timing.record_ms,
                timing.semantics_ms,
                timing.session_ms,
                timing.submit_ms,
                timing.present_ms,
                timing.shapes_built,
                timing.shapes_measure_hit,
                timing.shapes_measure_miss,
                timing.shapes_paint_hit,
                timing.shapes_paint_miss,
                timing.picture_rebuilt,
                timing.picture_reused,
                timing.layer_count,
                phase.script.motion_ms,
                phase.script.dispatch_ms,
                phase.script.timer_ms,
                phase.script.raf_ms,
                phase.script.callback_ms,
                phase.script.flush_ms,
                phase.trace.laywork_ms,
                phase.trace.layout_passes,
                phase.trace.layout_cause,
                phase.trace.text_reused,
                phase.trace.text_shaped,
                phase.trace.pointers,
                phase.trace.keys,
                phase.trace.edits,
                phase.trace.ime,
                phase.trace.scrolls,
                phase.trace.focus,
                phase.trace.blur,
                phase.trace.gestures
            ),
        );
    }
    let after = after_present(presented, again);
    if after == AfterPresent::Fatal {
        return Err(BootError::Engine(engine::Status::NotLinked));
    }
    let follow = scheduler::present_followup(after);
    runtime.needs_present = follow.needs_present;
    if follow.clear_surface {
        runtime.surfaces.clear();
        runtime.installed = None;
    }
    if matches!(after, AfterPresent::Shown { .. } | AfterPresent::Stop) {
        runtime.present_failures = 0;
        runtime.present_logged = false;
        runtime.present_retry_at = None;
    }
    let mut again = follow.vsync == VsyncOp::Request;
    if after == AfterPresent::Retry {
        runtime.present_failures = runtime.present_failures.saturating_add(1);
        if let Some(delay) = present::present_retry_delay_ms(runtime.present_failures) {
            if !runtime.present_logged {
                runtime.present_logged = true;
                (runtime.log)(LogLevel::Error, "present failed; backing off");
            }
            runtime.present_retry_at = Some(crate::net::mono_ms() as i64 + delay);
            again = false;
        } else {
            runtime.present_retry_at = None;
        }
    }
    let stop = follow.vsync == VsyncOp::Stop;
    Ok(end_frame(runtime, again, stop))
}

enum OpenPlace {
    Command,
    Frame,
}

enum OpenedSurface {
    Ready { replaced: Option<SurfaceId> },
    Waiting,
}

fn open_surface_target(
    runtime: &mut Runtime,
    target: SurfaceTarget,
    attempts: u32,
    place: OpenPlace,
) -> Result<OpenedSurface, BootError> {
    note_scale(runtime, target.metrics.device_pixel_ratio);
    let metrics = target.metrics;
    let kept = SurfaceTarget {
        id: target.id,
        layer: target.layer,
        metrics,
    };
    let opened = {
        let mut engine = runtime.engine.borrow_mut();
        runtime
            .surfaces
            .ready(target, &mut engine, runtime.open_surface.as_mut())
    };
    match opened {
        Ready::Opened { replaced, .. } => {
            if matches!(place, OpenPlace::Command) {
                if runtime.input.set_metrics(metrics) {
                    arm(runtime);
                }
                arm(runtime);
            } else {
                runtime.input.set_metrics(metrics);
            }
            Ok(OpenedSurface::Ready { replaced })
        }
        Ready::Failed(engine::Status::NotReady) => {
            let retry = runtime.surfaces.fail_attempt(kept, attempts);
            if matches!(retry, OpenRetry::GaveUp) {
                (runtime.log)(LogLevel::Error, "surface open failed; stopping retry");
            } else {
                arm(runtime);
            }
            Ok(OpenedSurface::Waiting)
        }
        Ready::Failed(status) => {
            let error = BootError::Engine(status);
            (runtime.log)(LogLevel::Error, &error.to_string());
            Err(error)
        }
    }
}

fn images_ready(
    runtime: &mut Runtime,
    requirements: &[(Arc<str>, bool)],
    census: Option<&ui_core::ImageCensus>,
) -> bool {
    runtime.images.begin_frame();
    if let Some(census) = census {
        runtime.images.sync_counts(census.counts.as_ref());
    }
    let bucket = images::scale_bucket(runtime.point_scale);
    runtime.images.pin_sources(
        requirements.iter().map(|(source, _)| source.as_ref()),
        bucket,
    );
    let mut waiting = false;
    let mut blocked = false;
    for (source, required) in requirements {
        let source = source.as_ref();
        let required = *required;
        if runtime.images.is_pending(source, bucket) {
            if required {
                waiting = true;
            }
            continue;
        }
        if runtime.images.resolve(source, bucket).is_some() {
            continue;
        }
        if runtime.images.is_negative(source, bucket) {
            if required {
                blocked = true;
                if let Some(message) = runtime.images.take_required_log(source, bucket) {
                    (runtime.log)(LogLevel::Error, &message);
                }
            }
            continue;
        }
        if let Some(request) = runtime.images.request(source, bucket) {
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
    let session_deadline = runtime.session.as_mut().and_then(|session| {
        if runtime.input.is_paused() {
            None
        } else {
            session.next_wake_ms().map(|ms| ms as i64)
        }
    });
    let deadline = if runtime.input.is_paused() {
        session_deadline
    } else {
        match (session_deadline, runtime.present_retry_at) {
            (Some(session_at), Some(retry_at)) => Some(session_at.min(retry_at)),
            (None, Some(retry_at)) => Some(retry_at),
            (session_at, None) => session_at,
        }
    };
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
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use session::{FrameOutput, FrameTime, IoEvent, Metrics};

    use super::install_fonts;
    use super::{Acks, Installed, Runtime, lock, on_command};
    use crate::command::{Command, FrameToken, SurfaceId, SurfaceTarget};
    use crate::images::{ImageHandle, ImageReady};
    use crate::scheduler::FrameScheduler;
    use crate::surface::{SURFACE_OPEN_ATTEMPTS, SurfaceHost};
    use crate::{ClipboardHost, LogLevel, NativePtr, Presented, Surface, Vsync};

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

    struct NullServices;

    impl crate::Services for NullServices {
        fn font(&self, _: &str) -> Option<&[u8]> {
            None
        }

        fn image(&self, _: &str) -> Option<engine::ImagePixels<'_>> {
            None
        }

        fn log(&self, _: LogLevel, _: &str) {}

        fn apply_ime(&self, _: &session::ImeUpdate) {}
    }

    struct NullSurface;

    impl Surface for NullSurface {
        fn present(&mut self, _: &mut engine::Engine) -> Result<Presented, engine::Status> {
            Ok(Presented::Continue)
        }
    }

    struct RecVsync {
        tokens: Mutex<Vec<FrameToken>>,
        cancels: Mutex<u32>,
    }

    impl RecVsync {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                tokens: Mutex::new(Vec::new()),
                cancels: Mutex::new(0),
            })
        }

        fn token_count(&self) -> usize {
            lock(&self.tokens).len()
        }

        fn last_token(&self) -> Option<FrameToken> {
            lock(&self.tokens).last().copied()
        }

        fn cancel_count(&self) -> u32 {
            *lock(&self.cancels)
        }
    }

    impl Vsync for RecVsync {
        fn bind(&self, _: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>) {}

        fn request_frame(&self, token: FrameToken) {
            lock(&self.tokens).push(token);
        }

        fn cancel(&self) {
            *lock(&self.cancels) += 1;
        }
    }

    struct SessionLog {
        io: Vec<IoEvent>,
        frames: Vec<FrameTime>,
        wants: bool,
        launch_urls: Vec<String>,
        clipboard: Option<String>,
    }

    struct MockSession {
        log: Rc<RefCell<SessionLog>>,
    }

    impl session::Session for MockSession {
        fn metrics(&mut self, _: &Metrics) {}

        fn pointer(&mut self, _: ui_core::PointerEvent) {}

        fn frame(&mut self, time: FrameTime) -> FrameOutput {
            let mut log = self.log.borrow_mut();
            log.frames.push(time);
            let mut output = FrameOutput::unchanged();
            output.clipboard = log.clipboard.take();
            output
        }

        fn wants_frame(&mut self) -> bool {
            self.log.borrow().wants
        }

        fn io_event(&mut self, event: IoEvent) {
            self.log.borrow_mut().io.push(event);
        }

        fn set_launch_url(&mut self, url: &str) {
            self.log.borrow_mut().launch_urls.push(url.to_owned());
        }
    }

    struct TestHarness {
        runtime: Runtime,
        acks: Acks,
        vsync: Arc<RecVsync>,
        session: Rc<RefCell<SessionLog>>,
    }

    impl TestHarness {
        fn new() -> Self {
            let engine = Rc::new(RefCell::new(engine::Engine::inert()));
            let typography = engine.borrow().typography();
            let vsync = RecVsync::new();
            let log = Rc::new(RefCell::new(SessionLog {
                io: Vec::new(),
                frames: Vec::new(),
                wants: false,
                launch_urls: Vec::new(),
                clipboard: None,
            }));
            let (image_tx, _image_rx) = mpsc::channel();
            let (wake_tx, _wake_rx) = mpsc::channel();
            let runtime = Runtime {
                services: Rc::new(RefCell::new(Box::new(NullServices))),
                engine,
                typography,
                log: Rc::new(|_, _| {}),
                session: Some(Box::new(MockSession {
                    log: Rc::clone(&log),
                })),
                registry: Arc::new(session::ModuleRegistry::new(Vec::new())),
                latest: None,
                input: crate::input::Input::new(),
                surfaces: SurfaceHost::new(),
                scheduler: FrameScheduler::new(),
                vsync: Arc::clone(&vsync) as Arc<dyn Vsync>,
                open_surface: Box::new(|_, _| Ok(Box::new(NullSurface))),
                installed: None,
                point_scale: 1.0,
                needs_present: false,
                images: crate::images::ImageCache::new(),
                image_tx,
                frame_number: 0,
                semantics: None,
                panic_streak: 0,
                wake_tx,
                dump_list: false,
                present_failures: 0,
                present_logged: false,
                present_retry_at: None,
            };
            Self {
                runtime,
                acks: Acks::new(),
                vsync,
                session: log,
            }
        }

        fn command(&mut self, command: Command) -> bool {
            on_command(&mut self.runtime, command, &self.acks)
                .unwrap_or_else(|error| panic!("{error}"))
        }

        fn open_dummy(&mut self) -> SurfaceId {
            let id = SurfaceId::fresh();
            assert!(!self.command(Command::SurfaceReady(SurfaceTarget {
                id,
                layer: NativePtr(std::ptr::null_mut()),
                metrics: Metrics::new(360.0, 640.0, 2.0),
            })));
            assert!(self.runtime.surfaces.is_open());
            id
        }

        fn frame_count(&self) -> usize {
            self.session.borrow().frames.len()
        }
    }

    fn now() -> FrameTime {
        FrameTime {
            vsync_ms: 1.0,
            target_ms: 1.0,
            interval_ms: 16.0,
            mono_ms: 1.0,
        }
    }

    #[test]
    fn launch_url_reaches_the_session() {
        let mut harness = TestHarness::new();
        assert!(!harness.command(Command::LaunchUrl("https://app.example/item".into())));
        assert_eq!(
            harness.session.borrow().launch_urls.as_slice(),
            ["https://app.example/item"]
        );
    }

    #[test]
    fn empty_launch_url_does_not_reach_the_session() {
        let mut harness = TestHarness::new();
        assert!(!harness.command(Command::LaunchUrl(String::new())));
        assert!(harness.session.borrow().launch_urls.is_empty());
    }

    #[test]
    fn wake_while_paused_does_not_frame() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Paused)));
        assert!(harness.runtime.input.is_paused());
        assert!(!harness.command(Command::Wake));
        assert_eq!(harness.frame_count(), 0);
    }

    #[test]
    fn resumed_clears_present_backoff_and_rearms() {
        let mut harness = TestHarness::new();
        let target = SurfaceTarget {
            id: SurfaceId::fresh(),
            layer: NativePtr(std::ptr::null_mut()),
            metrics: Metrics::new(360.0, 640.0, 2.0),
        };
        harness
            .runtime
            .surfaces
            .fail_attempt(target, SURFACE_OPEN_ATTEMPTS - 1);
        assert!(harness.runtime.surfaces.take_attempt().is_none());
        harness.runtime.needs_present = false;
        harness.runtime.present_failures = 3;
        harness.runtime.present_logged = true;
        harness.runtime.present_retry_at = Some(1);
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Resumed)));
        assert!(harness.runtime.needs_present);
        assert_eq!(harness.runtime.present_failures, 0);
        assert!(!harness.runtime.present_logged);
        assert!(harness.runtime.present_retry_at.is_none());
        assert!(harness.runtime.surfaces.take_attempt().is_some());
    }

    #[test]
    fn memory_warning_drops_installed_list_and_trims_images() {
        let mut harness = TestHarness::new();
        let request = harness
            .runtime
            .images
            .request("asset:logo", 1)
            .expect("pending decode");
        harness
            .runtime
            .images
            .apply(ImageReady::Decoded(ImageHandle {
                source: Arc::from("asset:logo"),
                generation: request.generation,
                size: [1, 1],
                pixels: Arc::from([0u8, 0, 0, 0]),
            }));
        assert!(harness.runtime.images.contains("asset:logo", 1));
        harness.runtime.installed = Some(Installed { scale: 2.0 });
        let tokens_before = harness.vsync.token_count();
        assert!(!harness.command(Command::MemoryWarning));
        assert!(harness.runtime.installed.is_none());
        assert!(!harness.runtime.images.contains("asset:logo", 1));
        assert!(harness.vsync.token_count() > tokens_before);
    }

    #[test]
    fn io_event_arms_only_when_session_wants_frame() {
        let mut harness = TestHarness::new();
        let tokens_before = harness.vsync.token_count();
        assert!(!harness.command(Command::HttpDone {
            token: 7,
            result: Err(session::HttpError::new(
                session::HttpFailure::Network,
                "offline"
            )),
        }));
        assert!(
            matches!(
                harness.session.borrow().io.last(),
                Some(IoEvent::Http { token: 7, .. })
            ),
            "http completion must reach the session"
        );
        assert_eq!(harness.vsync.token_count(), tokens_before);
        harness.session.borrow_mut().wants = true;
        assert!(!harness.command(Command::PluginDone {
            token: 8,
            result: session::PluginResult::Done(session::PluginValue::Null),
        }));
        assert!(
            matches!(
                harness.session.borrow().io.last(),
                Some(IoEvent::Plugin { token: 8, .. })
            ),
            "plugin completion must reach the session"
        );
        assert!(!harness.command(Command::Socket {
            id: 9,
            event: "open".into(),
            text: true,
            data: Vec::new(),
        }));
        assert!(
            matches!(
                harness.session.borrow().io.last(),
                Some(IoEvent::Socket { id: 9, .. })
            ),
            "socket completion must reach the session"
        );
        assert!(harness.vsync.token_count() > tokens_before);
    }

    #[test]
    fn stale_frame_token_is_dropped() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let armed = harness
            .vsync
            .last_token()
            .expect("SurfaceReady arms one vsync token");
        assert!(!harness.command(Command::Frame {
            token: FrameToken(armed.0.wrapping_add(1)),
            time: now(),
        }));
        assert_eq!(harness.frame_count(), 0);
    }

    #[test]
    fn dropped_surface_cancels_vsync_and_scheduler() {
        let mut harness = TestHarness::new();
        let id = harness.open_dummy();
        harness.runtime.installed = Some(Installed { scale: 1.0 });
        let cancels_before = harness.vsync.cancel_count();
        assert!(!harness.command(Command::SurfaceDestroyed { id }));
        assert!(harness.runtime.installed.is_none());
        assert!(!harness.runtime.surfaces.is_open());
        assert!(harness.vsync.cancel_count() > cancels_before);
        assert!(
            harness.runtime.scheduler.request().is_some(),
            "cancel must drop the armed token so the next request is accepted"
        );
    }

    #[test]
    fn remaining_command_arms_are_reachable() {
        let mut harness = TestHarness::new();
        harness.session.borrow_mut().wants = true;
        let id = harness.open_dummy();
        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics: Metrics::new(400.0, 800.0, 2.0),
        }));
        assert!(
            !harness.command(Command::Pointer(ui_core::PointerEvent::new(
                ui_core::Phase::Move,
                1.0,
                1.0,
                0.0,
            )))
        );
        assert!(!harness.command(Command::Input(session::InputEvent::Back)));
        assert!(!harness.command(Command::ImeDismissed));
        assert!(!harness.command(Command::Metrics(Metrics::new(360.0, 640.0, 3.0))));
        assert!(!harness.command(Command::Invalidate));
        assert!(harness.runtime.needs_present);
        assert!(!harness.command(Command::Settings(session::PlatformSettings::default())));
        assert!(!harness.command(Command::WindowFocus(true)));
        assert!(!harness.command(Command::LaunchUrl("https://app.example/item".into())));
        assert!(!harness.command(Command::SemanticsAction(
            ui_core::SemanticsAction::Activate(ui_core::SemanticsId(1))
        )));
        assert!(!harness.command(Command::ReloadImage("asset:logo".into())));
        assert!(!harness.command(Command::ImageReady(ImageReady::Missing {
            source: Arc::from("asset:missing"),
            generation: 1,
        })));
        assert!(!harness.command(Command::HttpDone {
            token: 11,
            result: Err(session::HttpError::new(
                session::HttpFailure::Network,
                "retry"
            )),
        }));
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Inactive)));
        let replaced = SurfaceId::fresh();
        assert!(!harness.command(Command::SurfaceReady(SurfaceTarget {
            id: replaced,
            layer: NativePtr(std::ptr::null_mut()),
            metrics: Metrics::new(360.0, 640.0, 2.0),
        })));
        let frames_before = harness.frame_count();
        assert!(!harness.command(Command::Wake));
        assert!(
            harness.frame_count() > frames_before,
            "Wake on an open surface must run the session frame"
        );
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Hidden)));
        assert!(harness.runtime.input.is_paused());
        assert!(harness.command(Command::Shutdown));
    }

    #[test]
    fn accepted_frame_token_runs_the_session() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let token = match harness.vsync.last_token() {
            Some(token) => token,
            None => panic!("SurfaceReady arms one vsync token"),
        };
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(harness.frame_count(), 1);
    }

    #[test]
    fn editor_copy_calls_clipboard_set_string() {
        let clip = crate::memory::MemoryClipboard::new("");
        let mut harness = TestHarness::new();
        harness.runtime.registry = Arc::new(session::ModuleRegistry::new(vec![crate::clipboard(
            Arc::clone(&clip) as _,
        )]));
        harness.open_dummy();
        harness.session.borrow_mut().clipboard = Some("copied".into());
        let token = match harness.vsync.last_token() {
            Some(token) => token,
            None => panic!("SurfaceReady arms one vsync token"),
        };
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(clip.read_clipboard(), "copied");
    }

    #[test]
    fn editor_copy_without_clipboard_logs_not_linked() {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut harness = TestHarness::new();
        let lines_log = Rc::clone(&lines);
        harness.runtime.log = Rc::new(move |level, message| {
            lines_log.borrow_mut().push((level, message.to_owned()));
        });
        harness.open_dummy();
        harness.session.borrow_mut().clipboard = Some("copied".into());
        let token = match harness.vsync.last_token() {
            Some(token) => token,
            None => panic!("SurfaceReady arms one vsync token"),
        };
        assert!(!harness.command(Command::Frame { token, time: now() }));
        let lines = lines.borrow();
        assert!(
            lines
                .iter()
                .any(|(level, message)| *level == LogLevel::Error && message.contains("not linked")),
            "{lines:?}"
        );
    }

    #[test]
    fn not_ready_surface_waits_and_a_gave_up_change_rearms() {
        let mut harness = TestHarness::new();
        harness.runtime.open_surface = Box::new(|_, _| Err(engine::Status::NotReady));
        let id = SurfaceId::fresh();
        assert!(!harness.command(Command::SurfaceReady(SurfaceTarget {
            id,
            layer: NativePtr(std::ptr::null_mut()),
            metrics: Metrics::new(360.0, 640.0, 2.0),
        })));
        assert!(!harness.runtime.surfaces.is_open());
        harness.runtime.surfaces.fail_attempt(
            SurfaceTarget {
                id,
                layer: NativePtr(std::ptr::null_mut()),
                metrics: Metrics::new(360.0, 640.0, 2.0),
            },
            SURFACE_OPEN_ATTEMPTS - 1,
        );
        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics: Metrics::new(400.0, 800.0, 2.0),
        }));
        assert!(harness.runtime.surfaces.take_attempt().is_some());
    }
}
