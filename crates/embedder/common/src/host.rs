//! Render thread. Commands come from the platform thread. This thread owns the
//! engine, the session, and the surface.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use session::FrameTime;

use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::error::BootError;
use crate::images::{self, ImageCache, ImageDecoder, ImageError, ImageReady, ImageRequest};
use crate::input::{FrameInput, Input};
use crate::present::{self, AfterPresent, after_present};
use crate::queue::Inbox;
use crate::scheduler::{self, FrameScheduler, FrameStep, FrameView, Gpu, VsyncOp};

use crate::surface::{OpenRetry, Ready, SurfaceHost};
use crate::timing::FrameTiming;
use crate::{Boot, Launch, Services};

mod cache_cmds;
mod frame_cmds;
mod session_cmds;
mod surface_cmds;

pub fn console_log() -> crate::ConsoleLog {
    Box::new(|level, message| {
        // QuickJS console levels: 0 log, 1 info, 2 warn, 3 error, 4 debug.
        if message.starts_with("unhandled rejection:") {
            tracing::error!(target: "impasto::script::promise", "{message}");
            return;
        }
        match level {
            3 => tracing::error!(target: "impasto::script::console", "{message}"),
            2 => tracing::warn!(target: "impasto::script::console", "{message}"),
            4 => tracing::debug!(target: "impasto::script::console", "{message}"),
            _ => tracing::info!(target: "impasto::script::console", "{message}"),
        }
    })
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
    state: Mutex<AckState>,
    cv: Condvar,
}

#[derive(Default)]
struct AckState {
    pending: HashMap<SurfaceId, bool>,
    closed: bool,
}

impl Acks {
    fn new() -> Self {
        Self {
            state: Mutex::new(AckState::default()),
            cv: Condvar::new(),
        }
    }

    fn arm(&self, id: SurfaceId) -> bool {
        let mut state = lock(&self.state);
        if state.closed {
            return false;
        }
        state.pending.entry(id).or_insert(false);
        true
    }

    fn signal(&self, id: SurfaceId) {
        if let Some(done) = lock(&self.state).pending.get_mut(&id) {
            *done = true;
        }
        self.cv.notify_all();
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.pending.clear();
        self.cv.notify_all();
    }

    fn poll(&self, id: SurfaceId) -> bool {
        let mut state = lock(&self.state);
        if state.closed || state.pending.get(&id) == Some(&true) {
            state.pending.remove(&id);
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
        let mut state = lock(&self.acks.state);
        loop {
            match state.pending.get(&id).copied() {
                Some(true) => {
                    state.pending.remove(&id);
                    return;
                }
                None => return,
                Some(false) => {
                    state = self
                        .acks
                        .cv
                        .wait(state)
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
    cancelled: Arc<AtomicBool>,
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
        if self.acks.arm(id) {
            self.inbox.post(Command::SurfaceDestroyed { id });
        }
    }

    pub fn poll_dropped(&self, id: SurfaceId) -> bool {
        self.acks.poll(id)
    }

    /// A waiter for [`Self::begin_destroy`] when the platform requires synchronous teardown.
    pub fn drop_wait(&self) -> DropWait {
        DropWait {
            acks: Arc::clone(&self.acks),
        }
    }

    /// Requests shutdown without waiting for the render thread. A caller owning
    /// a published native surface must acknowledge its destruction separately.
    pub fn request_shutdown(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            self.inbox.post(Command::Shutdown);
        }
    }

    pub fn shutdown_and_join(&self) -> Result<(), BootError> {
        self.request_shutdown();
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

impl Drop for Link {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

struct ExitGuard {
    done: Option<Sender<Result<(), BootError>>>,
    acks: Arc<Acks>,
    inbox: Arc<Inbox>,
}

impl ExitGuard {
    fn finish(mut self, result: Result<(), BootError>) {
        self.acks.close();
        self.inbox.close();
        if let Some(done) = self.done.take() {
            let _ = done.send(result);
        }
    }
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            self.acks.close();
            self.inbox.close();
            let _ = done.send(Err(BootError::Panicked));
        }
    }
}

/// Panics reach the hub before the default hook so Android logcat and the
/// devtools panel keep the payload. The first hub installed wins; there is
/// one hub per process.
fn install_panic_hook(hub: Arc<crate::diagnostics::DiagnosticsHub>) {
    static INSTALLED: std::sync::OnceLock<Option<Arc<crate::diagnostics::DiagnosticsHub>>> =
        std::sync::OnceLock::new();
    if INSTALLED.set(Some(Arc::clone(&hub))).is_err() {
        return;
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(hub) = INSTALLED.get().and_then(|slot| slot.as_ref()) {
            let payload = format!("{info}");
            let backtrace = std::backtrace::Backtrace::force_capture().to_string();
            hub.error(
                "host",
                &format!("panic: {payload}"),
                Some(backtrace.trim()),
                None,
            );
        }
        default(info);
    }));
}

fn host_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    String::new()
}

/// Writes `frame.json` once, then asks the host to leave. A missing directory
/// or a failed write is logged and not retried.
fn issue_list(report: &str) -> &str {
    report
        .strip_prefix("{\"issues\":")
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or("[]")
}

fn write_inspect(
    runtime: &mut Runtime,
    tree: &uicore::LayerTree,
    timing: &FrameTiming,
    phases: &session::FramePhases,
    issues: &str,
) {
    if runtime.inspect_written {
        return;
    }
    let Some(dir) = runtime.inspect_dir.clone() else {
        return;
    };
    match crate::diagnostics::write_frame(&dir, tree, timing, Some(phases), issues) {
        Ok(()) => {
            runtime.inspect_written = true;
            runtime.services.borrow().inspect_finished();
        }
        Err(error) => {
            runtime.inspect_written = true;
            tracing::error!(target: "impasto::shell", %error, "inspect write failed");
        }
    }
}

pub fn start(boot: Boot) -> Result<Link, BootError> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let link = start_async(
        move || Ok(boot),
        move |result| {
            let _ = ready_tx.send(result);
        },
    )?;
    ready_rx.recv().map_err(|_| BootError::Panicked)??;
    Ok(link)
}

type StartupCallback = Box<dyn FnOnce(Result<(), BootError>) + Send>;

struct Startup {
    cancelled: Arc<AtomicBool>,
    notify: Option<StartupCallback>,
}

impl Startup {
    fn checkpoint(&self) -> Result<(), BootError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(BootError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn report(&mut self, result: Result<(), BootError>) {
        if let Some(notify) = self.notify.take() {
            notify(result);
        }
    }
}

/// Creates the command link immediately. Preparation and the one-shot readiness
/// callback run on the render thread. Cancellation is observed at startup
/// checkpoints, so an in-progress platform call may still need to return.
pub fn start_async(
    prepare: impl FnOnce() -> Result<Boot, BootError> + Send + 'static,
    on_started: impl FnOnce(Result<(), BootError>) + Send + 'static,
) -> Result<Link, BootError> {
    start_async_with_exit(prepare, on_started, |_| {})
}

/// Like [`start_async`], with a notification after render resources are
/// released and the inbox is closed. Both callbacks run on the render thread.
pub fn start_async_with_exit(
    prepare: impl FnOnce() -> Result<Boot, BootError> + Send + 'static,
    on_started: impl FnOnce(Result<(), BootError>) + Send + 'static,
    on_exit: impl FnOnce(Result<(), BootError>) + Send + 'static,
) -> Result<Link, BootError> {
    let inbox = Arc::new(Inbox::new());
    let inbox_thread = Arc::clone(&inbox);
    let (done_tx, done_rx) = mpsc::channel();
    let acks = Arc::new(Acks::new());
    let acks_thread = Arc::clone(&acks);
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut startup = Startup {
        cancelled: Arc::clone(&cancelled),
        notify: Some(Box::new(on_started)),
    };
    thread::Builder::new()
        .name("render".to_string())
        .spawn(move || {
            let guard = ExitGuard {
                done: Some(done_tx),
                acks: Arc::clone(&acks_thread),
                inbox: Arc::clone(&inbox_thread),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                startup.checkpoint()?;
                let boot = prepare();
                startup.checkpoint()?;
                run_boot(boot?, &inbox_thread, &acks_thread, &mut startup)
            }))
            .unwrap_or(Err(BootError::Panicked));
            guard.finish(result.clone());
            startup.report(result.clone());
            on_exit(result);
        })
        .map_err(|error| BootError::Platform(error.to_string()))?;
    Ok(Link {
        inbox,
        done: Mutex::new(Some(done_rx)),
        acks,
        cancelled,
    })
}

fn run_boot(
    boot: Boot,
    inbox: &Arc<Inbox>,
    acks: &Acks,
    startup: &mut Startup,
) -> Result<(), BootError> {
    crate::log_sub::install(boot.log_dir.as_deref(), boot.devtools.clone());
    let (image_tx, image_rx) = mpsc::channel();
    let upload = Arc::new(Mutex::new(None));
    if let Some(hub) = boot.devtools.as_ref() {
        install_panic_hook(Arc::clone(hub));
    }
    let http = boot
        .http
        .clone()
        .map(|inner| crate::diagnostics::wrap_if_devtools(inner, boot.devtools.clone()));
    let decoder = Arc::clone(&boot.decoder);
    let inbox_images = Arc::clone(inbox);
    let http_images = http.clone();
    let upload_io = Arc::clone(&upload);
    let _io = thread::Builder::new()
        .name("io".to_string())
        .spawn(move || {
            decode_loop(decoder, image_rx, &inbox_images, http_images, &upload_io);
        });
    let (net_tx, net_rx) = mpsc::sync_channel(64);
    let (wake_tx, wake_rx) = mpsc::channel();
    let inbox_net = Arc::clone(inbox);
    let _net = crate::net::spawn(
        net_rx,
        inbox_net,
        http,
        boot.services.websocket_transport(),
        boot.devtools.clone(),
    );
    let _time = crate::net::wake(wake_rx, Arc::clone(inbox));
    render(
        boot,
        inbox,
        acks,
        startup,
        RenderLinks {
            image_tx,
            net_tx,
            wake_tx,
            upload,
        },
    )
}

fn decode_loop(
    decoder: Arc<dyn ImageDecoder>,
    jobs: Receiver<ImageRequest>,
    inbox: &Inbox,
    http: Option<Arc<dyn session::HttpClient>>,
    upload: &Mutex<Option<engine::TextureFactory>>,
) {
    while let Ok(request) = jobs.recv() {
        inbox.post(Command::ImageReady(decode_one(
            decoder.as_ref(),
            http.as_deref(),
            &request,
            upload,
        )));
    }
}

fn decode_one(
    decoder: &dyn ImageDecoder,
    http: Option<&dyn session::HttpClient>,
    request: &ImageRequest,
    upload: &Mutex<Option<engine::TextureFactory>>,
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
            {
                let factory = lock(upload);
                images::attach_upload(&mut handle, factory.as_ref());
            }
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
    session: Option<Box<dyn session::Session>>,
    net_tx: SyncSender<session::IoJob>,
    registry: Arc<session::ModuleRegistry>,
    latest: Option<Arc<protocol::LayerTree>>,
    image_requirements: Vec<(Arc<str>, bool)>,
    external_textures: HashMap<String, images::ExternalFrame>,
    external_gpu: HashMap<String, images::ExternalGpuFrame>,
    input: Input,
    surfaces: SurfaceHost,
    scheduler: FrameScheduler,
    vsync: Arc<dyn crate::Vsync>,
    open_surface: crate::OpenSurface,
    installed: Option<Installed>,
    point_scale: f32,
    needs_present: bool,
    images: ImageCache,
    image_tx: Sender<ImageRequest>,
    upload: Arc<Mutex<Option<engine::TextureFactory>>>,
    frame_number: u64,
    semantics: Option<uicore::SemanticsUpdate>,
    semantics_enabled: bool,
    autofill_enabled: bool,
    platform_views: Arc<[uicore::PlatformViewPlacement]>,
    platform_views_surface: Option<SurfaceId>,
    platform_views_dirty: bool,
    wake_tx: Sender<Option<i64>>,
    dump_list: bool,
    present_failures: u32,
    device_recoveries: u8,
    present_retry_at: Option<i64>,
    gpu: Gpu,
    last_step: Option<(FrameStep, Gpu)>,
    listeners: Vec<Box<dyn crate::timing::FrameListener>>,
    inspect: Option<std::path::PathBuf>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
    inspect_dir: Option<std::path::PathBuf>,
    inspect_written: bool,
    busy: Arc<crate::watchdog::Busy>,
    /// One extra frame was already armed for the latest script fault.
    diag_armed: bool,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.vsync.cancel();
        // Vulkan swapchains use the engine context. Retire them before its fields drop.
        self.surfaces.clear();
        // The shaper shares the engine typography context. Drop it before the engine.
        self.session.take();
    }
}

struct RenderLinks {
    image_tx: Sender<ImageRequest>,
    net_tx: SyncSender<session::IoJob>,
    wake_tx: Sender<Option<i64>>,
    upload: Arc<Mutex<Option<engine::TextureFactory>>>,
}

fn render(
    boot: Boot,
    inbox: &Arc<Inbox>,
    acks: &Acks,
    startup: &mut Startup,
    links: RenderLinks,
) -> Result<(), BootError> {
    startup.checkpoint()?;
    let services = Rc::new(RefCell::new(boot.services));
    let engine = {
        let services = services.borrow();
        prepare_engine(services.as_ref(), &boot.font_family, boot.open_engine)
    };
    startup.checkpoint()?;
    let engine = engine?;
    let Launch::Session(open) = boot.launch;
    let parts = crate::SessionOpen {
        shaper: Box::new(crate::ImpellerShaper::new(engine.borrow().typography())),
        log: console_log(),
    };
    let rt = match crate::rt::handle() {
        Ok(handle) => handle,
        Err(message) => {
            tracing::error!(target: "impasto::session", "{message}");
            return Err(crate::SessionError::Runtime.into());
        }
    };
    let app_state = session::shared("active".to_owned());
    let launch_url = Arc::clone(&boot.launch_url);
    let window_session = services.borrow().window_session();
    let window = Arc::new(Poster {
        inbox: Arc::clone(inbox),
    });
    window_session.connect(&window);
    let mut modules = vec![crate::modules::app_state::with_restoration(
        Arc::clone(&app_state),
        services.borrow().restoration(),
        window_session,
    )];
    if let Some(hub) = boot.devtools.as_ref() {
        modules.push(crate::diagnostics::module(
            Arc::clone(hub),
            crate::diagnostics::DevtoolsContext {
                enabled: true,
                source_map: boot.devtools_source_map.clone(),
                app_state: Arc::clone(&app_state),
                os: std::env::consts::OS,
                os_version: host_os_version(),
                dump: std::env::var("IMPASTO_DUMP").ok(),
            },
        ));
    }
    modules.extend(boot.modules);
    let registry = Arc::new(session::ModuleRegistry::new(modules));
    let host = session::ModuleConfig {
        registry: Arc::clone(&registry),
        exec: Arc::new(crate::RtExec {
            rt,
            main: boot.main_thread.clone(),
        }),
        settle: crate::settle_factory(Arc::clone(inbox)),
        post_delivery: Some(Arc::new({
            let inbox = Arc::clone(inbox);
            move |delivery| inbox.post(Command::JsIo(delivery))
        })),
        transport_clock: Some(Arc::new(|| crate::net::ms_i64(crate::net::mono_ms()))),
        app_state,
        launch_url,
    };
    startup.checkpoint()?;
    let session = open(parts, io_sink(links.net_tx.clone()), host);
    startup.checkpoint()?;
    let session = match session {
        Ok(opened) => Some(opened),
        Err(error) => {
            let error = BootError::Session(error);
            tracing::error!(target: "impasto::session", %error, "session did not open");
            return Err(error);
        }
    };
    let inbox_frames = Arc::clone(inbox);
    boot.vsync.bind(Arc::new(move |token, mut time| {
        time.mono_ms = crate::net::mono_ms();
        inbox_frames.post(Command::Frame { token, time });
    }));
    let busy = Arc::new(crate::watchdog::Busy::new());
    crate::watchdog::spawn(Arc::downgrade(&busy), boot.devtools.clone());
    let mut runtime = Runtime {
        services,
        engine,
        session,
        net_tx: links.net_tx,
        registry,
        latest: None,
        image_requirements: Vec::new(),
        external_textures: Default::default(),
        external_gpu: Default::default(),
        input: Input::new(),
        surfaces: SurfaceHost::new(),
        scheduler: FrameScheduler::new(),
        vsync: boot.vsync,
        open_surface: boot.open_surface,
        installed: None,
        point_scale: 1.0,
        needs_present: false,
        images: ImageCache::new(),
        image_tx: links.image_tx,
        upload: links.upload,
        frame_number: 0,
        semantics: None,
        semantics_enabled: true,
        autofill_enabled: false,
        platform_views: Arc::default(),
        platform_views_surface: None,
        platform_views_dirty: false,
        wake_tx: links.wake_tx,
        dump_list: boot.dump_list,
        present_failures: 0,
        device_recoveries: 0,
        present_retry_at: None,
        gpu: Gpu::Live,
        last_step: None,
        listeners: frame_listeners(boot.devtools.clone()),
        inspect: boot.inspect,
        hub: boot.devtools.clone(),
        inspect_dir: boot.inspect_dir,
        inspect_written: false,
        busy,
        diag_armed: false,
    };
    startup.checkpoint()?;
    startup.report(Ok(()));
    publish_back_ownership(&mut runtime);
    while let Some(command) = inbox.recv() {
        match on_command(&mut runtime, command, acks) {
            Ok(true) => break,
            Ok(false) => publish_back_ownership(&mut runtime),
            Err(error) => {
                tracing::error!(target: "impasto::render", %error, "render thread stopped");
                runtime.services.borrow().request_exit();
                return Err(error);
            }
        }
    }
    Ok(())
}

fn io_sink(tx: SyncSender<session::IoJob>) -> crate::IoSink {
    Box::new(move |job| {
        tx.try_send(job).map_err(|error| match error {
            mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job) => Box::new(job),
        })
    })
}

fn update_source(runtime: &mut Runtime, revision: &str, source: &str) {
    let Some(session) = runtime.session.as_mut() else {
        return;
    };
    let restoration = runtime.services.borrow().restoration();
    let staged = restoration.as_ref().map(crate::Restoration::stage_update);
    let now = crate::net::mono_ms();
    let result = session.update_source(
        source,
        Box::new(crate::ImpellerShaper::new(
            runtime.engine.borrow().typography(),
        )),
        io_sink(runtime.net_tx.clone()),
        FrameTime {
            vsync_ms: now,
            target_ms: now,
            interval_ms: 16.0,
            mono_ms: now,
        },
    );
    if result.is_ok() {
        if let Some(update) = staged {
            update.commit();
        }
        runtime.input.reset_for_source_update();
        runtime.engine.borrow_mut().invalidate_pictures();
        runtime.installed = None;
        runtime.semantics = None;
        runtime.diag_armed = false;
        let services = runtime.services.borrow();
        services.apply_ime(&session::ImeUpdate::Hide);
        services.semantics(&[]);
        if let Some(surface) = runtime.surfaces.id() {
            services.platform_views(surface, &[]);
        }
        drop(services);
        arm(runtime);
    }
    runtime.services.borrow().source_updated(
        revision,
        match &result {
            Ok(()) => Ok(()),
            Err(error) => Err(error.as_str()),
        },
    );
}

fn publish_back_ownership(runtime: &mut Runtime) {
    if let Some(session) = runtime.session.as_mut() {
        session_call(|| {
            if let Some(enabled) = session.take_back_enabled() {
                runtime.services.borrow().back_enabled(enabled);
            }
        });
    }
}

pub(super) fn session_call(body: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
        tracing::error!(target: "impasto::script", "session call panicked");
    }
}

struct FaultPlan {
    diag_armed: bool,
    arm_extra: bool,
    stop_again: bool,
}

fn fault_followup(diag_armed: bool, faults: &[session::ScriptFault]) -> FaultPlan {
    if faults.is_empty() {
        return FaultPlan {
            diag_armed: false,
            arm_extra: false,
            stop_again: false,
        };
    }
    let held = faults
        .iter()
        .any(|fault| fault.source != "listener" && fault.source != "rejection");
    if !diag_armed {
        FaultPlan {
            diag_armed: true,
            arm_extra: true,
            stop_again: false,
        }
    } else {
        FaultPlan {
            diag_armed: true,
            arm_extra: false,
            stop_again: held,
        }
    }
}

fn on_command(runtime: &mut Runtime, command: Command, acks: &Acks) -> Result<bool, BootError> {
    runtime.busy.enter(crate::command::command_tag(&command));
    let result = dispatch_command(runtime, command, acks);
    runtime.busy.leave();
    result
}

fn dispatch_command(
    runtime: &mut Runtime,
    command: Command,
    acks: &Acks,
) -> Result<bool, BootError> {
    match command {
        Command::Shutdown => Ok(true),
        Command::UpdateSource { revision, source } => {
            update_source(runtime, &revision, &source);
            Ok(false)
        }
        command @ (Command::SurfaceDestroyed { .. }
        | Command::SurfaceReady(_)
        | Command::SurfaceChanged { .. }) => surface_cmds::handle(runtime, command, acks),
        command @ (Command::Pointer(_)
        | Command::Input(_)
        | Command::BackGesture(_)
        | Command::ImeDismissed
        | Command::Metrics(_)
        | Command::WindowDisplay(_)
        | Command::WindowStateChanged(_)
        | Command::Lifecycle(_)
        | Command::Settings(_)
        | Command::WindowFocus(_)
        | Command::SemanticsEnabled(_)
        | Command::AutofillEnabled(_)
        | Command::SemanticsAction(_)
        | Command::LaunchUrl(_)
        | Command::JsIo(_)
        | Command::HttpDone { .. }
        | Command::HttpHead { .. }
        | Command::HttpChunk { .. }
        | Command::PluginDone { .. }
        | Command::Socket { .. }) => session_cmds::handle(runtime, command),
        command @ (Command::Invalidate
        | Command::MemoryWarning
        | Command::ImageReady(_)
        | Command::ExternalTexture { .. }
        | Command::ExternalGpuTexture { .. }
        | Command::ReloadImage(_)) => cache_cmds::handle(runtime, command),
        command @ (Command::Wake | Command::Frame { .. }) => {
            frame_cmds::handle(runtime, command, acks)
        }
    }
}

fn fold_semantics(snapshot: &mut Option<uicore::SemanticsUpdate>, patch: &uicore::SemanticsPatch) {
    let update = snapshot.get_or_insert_with(|| uicore::SemanticsUpdate {
        root: patch.root,
        nodes: Vec::new(),
    });
    update.root = patch.root;
    for op in &patch.ops {
        match op {
            uicore::SemanticsOp::Upsert(node) => {
                if let Some(found) = update.nodes.iter_mut().find(|item| item.id == node.id) {
                    *found = node.clone();
                } else {
                    update.nodes.push(node.clone());
                }
            }
            uicore::SemanticsOp::Remove(id) => update.nodes.retain(|node| node.id != *id),
        }
    }
    if let Some(order) = &patch.order {
        let ranks: HashMap<_, _> = order.iter().copied().zip(0..).collect();
        update
            .nodes
            .sort_by_key(|node| ranks.get(&node.id).copied().unwrap_or(usize::MAX));
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
    use session::trace::Section;

    let _frame_trace = Section::begin(c"Impasto frame");
    let frame_started = Instant::now();
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
            session_call(|| session.metrics(&metrics));
        }
    }
    let gesture_started = Instant::now();
    let input_trace = Section::begin(c"Impasto input");
    let mut leave = false;
    if let Some(session) = runtime.session.as_mut() {
        for event in frame.events {
            match event {
                FrameInput::Pointer(event) => {
                    tracing::trace!(
                        target: "impasto::shell",
                        kind = phase_name(event.phase),
                        x = event.x,
                        y = event.y,
                        "pointer"
                    );
                    session_call(|| session.pointer(event));
                }
                FrameInput::Session(event) => {
                    tracing::trace!(target: "impasto::shell", kind = input_kind(&event), "input");
                    session_call(|| session.input(event));
                }
                FrameInput::DismissIme => {
                    tracing::trace!(target: "impasto::shell", "ime dismissed");
                    session_call(|| session.ime_dismissed());
                }
            }
        }
    }
    let gesture_ms = gesture_started.elapsed().as_secs_f64() * 1000.0;
    drop(input_trace);
    runtime.frame_number = runtime.frame_number.wrapping_add(1);
    Section::event(|| {
        if time.target_ms > time.vsync_ms {
            format!(
                "Impasto frame id={} vsync_us={:.0} target_us={:.0}",
                runtime.frame_number,
                time.vsync_ms * 1000.0,
                time.target_ms * 1000.0,
            )
        } else {
            format!("Impasto frame id={}", runtime.frame_number)
        }
    });
    let frame_span = tracing::error_span!(
        target: "impasto::render",
        "frame",
        frame = runtime.frame_number
    );
    let _entered = frame_span.enter();
    let session_started = Instant::now();
    let mut again;
    let mut reloads = Vec::new();
    let mut fonts = Vec::new();
    let mut census = None;
    let mut layers_changed = false;
    let mut phases = None;
    let mut layout_report = None;
    if let Some(session) = runtime.session.as_mut() {
        let framed = {
            let _trace = Section::begin(c"Impasto session");
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.frame(time)))
        };
        match framed {
            Ok(mut output) => {
                again = output.again;
                leave = output.exit;
                let faults = session.take_faults();
                let plan = fault_followup(runtime.diag_armed, &faults);
                runtime.diag_armed = plan.diag_armed;
                if plan.stop_again {
                    again = false;
                }
                if let Some(hub) = runtime.hub.as_ref() {
                    for fault in &faults {
                        hub.error(
                            fault.source,
                            &fault.message,
                            (!fault.stack.is_empty()).then_some(fault.stack.as_str()),
                            Some(runtime.frame_number),
                        );
                    }
                }
                if plan.arm_extra {
                    arm(runtime);
                }
                if let Some(tree) = output.layers.take() {
                    if runtime.dump_list {
                        tracing::debug!(
                            target: "impasto::render",
                            "{}",
                            layer_tree_line(&tree)
                        );
                    }
                    runtime.image_requirements = tree.image_requirements();
                    runtime.latest = Some(tree);
                    layers_changed = true;
                }
                if let Some(text) = output.clipboard.take()
                    && let Err(message) = runtime.registry.call(
                        "Clipboard",
                        "setString",
                        &[session::PluginValue::Str(text)],
                    )
                {
                    tracing::error!(target: "impasto::shell", "{message}");
                }
                if let Some(update) = output.ime.take() {
                    crate::ime::publish_update(&update, &runtime.services);
                }
                if let Some(patch) = output.semantics.take() {
                    let _trace = Section::begin(c"Impasto accessibility");
                    fold_semantics(&mut runtime.semantics, &patch);
                    if let Some(update) = runtime.semantics.as_ref() {
                        let services = runtime.services.borrow();
                        let projected = crate::semantics::project_with_platform_views(
                            update,
                            services.semantics_include_platform_views(),
                        );
                        services.semantics(&projected);
                    }
                }
                if let Some(placements) = output.platform_views.take() {
                    runtime.platform_views = placements;
                    runtime.platform_views_dirty = true;
                    runtime.needs_present = true;
                }
                reloads = std::mem::take(&mut output.reloads);
                fonts = std::mem::take(&mut output.fonts);
                census = output.images.take();
                phases = output.phases.take();
                layout_report = output.layout_report.take();
                if let Some(hub) = runtime.hub.as_ref()
                    && let Some(report) = layout_report.as_deref()
                {
                    hub.set_layout_report(report);
                }
            }
            Err(_) => {
                tracing::error!(
                    target: "impasto::script",
                    "session call panicked"
                );
                again = false;
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
        install_fonts(&mut runtime.engine.borrow_mut(), fonts);
    }
    let Some(tree) = runtime.latest.clone() else {
        return Ok(end_frame(runtime, again, false));
    };
    if !images_ready(runtime, census.as_deref()) {
        return Ok(end_frame(runtime, reloaded, false));
    }
    let unchanged = !layers_changed
        && runtime
            .installed
            .as_ref()
            .is_some_and(|installed| installed.scale == runtime.point_scale);
    let mut submit_ms = 0.0;
    let step = scheduler::frame_step(&FrameView {
        surface_open: runtime.surfaces.is_open(),
        paused: frame.paused,
        unchanged,
        needs_present: runtime.needs_present,
        again,
        gpu: runtime.gpu,
    });
    note_step(runtime, step);
    if again
        && matches!(step, FrameStep::Submit | FrameStep::PresentCached)
        && let Some(token) = runtime.scheduler.prepare_next()
    {
        runtime.vsync.request_frame(token);
    }
    match step {
        FrameStep::Cancel | FrameStep::Skip { again: false } => {
            let _trace = Section::begin(c"Impasto skip present");
            return Ok(end_frame(runtime, false, false));
        }
        FrameStep::Skip { again: true } => {
            let _trace = Section::begin(c"Impasto skip present");
            return Ok(end_frame(runtime, true, false));
        }
        FrameStep::Submit => {
            let _trace = Section::begin(c"Impasto submit");
            runtime.images.install_uploaded(
                &mut runtime.engine.borrow_mut(),
                images::scale_bucket(runtime.point_scale),
            );
            {
                let mut engine = runtime.engine.borrow_mut();
                for (source, frame) in &runtime.external_gpu {
                    if let Some(image) = &frame.image {
                        engine.install_uploaded(source, image);
                    }
                }
            }
            let started = Instant::now();
            let pixels = runtime.images.snapshot_requirements(
                &runtime.image_requirements,
                images::scale_bucket(runtime.point_scale),
            );
            let resolve = |source: &str| {
                if source.starts_with(protocol::EXTERNAL_TEXTURE_PREFIX) {
                    return runtime.external_textures.get(source).map(|frame| {
                        engine::ImagePixels {
                            width: frame.size[0],
                            height: frame.size[1],
                            rgba: &frame.pixels,
                        }
                    });
                }
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
                tracing::error!(
                    target: "impasto::render::present",
                    %status,
                    "scene submit failed"
                );
                if let Some(hub) = runtime.hub.as_ref() {
                    hub.error(
                        "engine",
                        &format!("scene submit failed: {status}"),
                        None,
                        Some(runtime.frame_number),
                    );
                }
                if status == engine::Status::NotLinked {
                    return Err(BootError::Engine(status));
                }
                // A rejected list is not a lost surface. Do not force another vsync.
                return Ok(end_frame(runtime, again, false));
            }
            runtime.installed = Some(Installed {
                scale: runtime.point_scale,
            });
            if runtime.gpu == Gpu::Flush {
                runtime.gpu = Gpu::Live;
                tracing::info!(target: "impasto::render::gpu", reason = "resume", "gpu live");
            }
        }
        FrameStep::PresentCached => {}
    }
    if runtime.gpu != Gpu::Live {
        return Ok(end_frame(runtime, false, false));
    }
    let started = Instant::now();
    let presented = {
        let _trace = Section::begin(c"Impasto present");
        let mut engine = runtime.engine.borrow_mut();
        runtime
            .surfaces
            .present(&mut engine, &runtime.platform_views)
    };
    if presented.is_ok() {
        let _trace = Section::begin(c"Impasto presented");
        if let Some(surface) = runtime.surfaces.id()
            && (runtime.platform_views_dirty
                || (!runtime.platform_views.is_empty()
                    && runtime.platform_views_surface != Some(surface)))
        {
            runtime
                .services
                .borrow()
                .platform_views(surface, &runtime.platform_views);
            runtime.platform_views_dirty = false;
            runtime.platform_views_surface = Some(surface);
        }
    }
    let present_ms = elapsed_ms(started);
    let work_ms = elapsed_ms(frame_started);
    let stats = runtime.engine.borrow().submit_stats();
    if matches!(step, FrameStep::Submit) {
        Section::event(|| {
            format!(
                "Impasto pictures rebuilt={} reused={} layers={} prepare_us={} compose_us={}",
                stats.picture_rebuilt,
                stats.picture_reused,
                stats.layer_count,
                stats.picture_prepare.as_micros(),
                stats.compose.as_micros(),
            )
        });
    }
    let shapes = runtime.engine.borrow_mut().take_shape_stats();
    let phase = phases.unwrap_or_default();
    let timing = FrameTiming {
        number: runtime.frame_number,
        vsync_ms: time.vsync_ms,
        target_ms: time.target_ms,
        interval_ms: time.interval_ms,
        work_ms,
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
        shapes_bridged: shapes.bridged,
        picture_rebuilt: stats.picture_rebuilt,
        picture_reused: stats.picture_reused,
        layer_count: stats.layer_count,
        paragraphs: stats.paragraphs,
        pictures: stats.pictures,
        textures: stats.textures,
    };
    for listener in &mut runtime.listeners {
        listener.on_frame(&timing, &phase);
    }
    // The richer `inspect_dir` writer owns `frame.json` when both are set.
    if runtime.inspect_dir.is_none()
        && let Some(dir) = &runtime.inspect
        && let Err(error) = crate::timing::write_inspect(dir, &timing, runtime.latest.as_deref())
    {
        tracing::warn!(target: "impasto::shell", %error, "inspect write failed");
    }
    let present_error = presented.as_ref().err().copied();
    if present_error == Some(engine::Status::DeviceLost) && runtime.device_recoveries < 2 {
        runtime.device_recoveries += 1;
        tracing::warn!(target: "impasto::render::gpu", attempts = runtime.device_recoveries, "graphics device lost, rebuilding");
        *lock(&runtime.upload) = None;
        runtime.images.invalidate_gpu();
        {
            let mut engine = runtime.engine.borrow_mut();
            runtime
                .surfaces
                .recover_device(&mut engine)
                .map_err(BootError::Engine)?;
            for frame in runtime.external_gpu.values_mut() {
                frame.image = (frame.import)(&engine);
            }
            *lock(&runtime.upload) = engine.texture_factory();
        }
        runtime.installed = None;
        runtime.gpu = Gpu::Flush;
        runtime.needs_present = true;
        return Ok(end_frame(runtime, true, false));
    }
    let after = after_present(presented, again);
    if after == AfterPresent::Fatal {
        return Err(BootError::Engine(
            present_error.unwrap_or(engine::Status::NotLinked),
        ));
    }
    if after == AfterPresent::Rejected {
        runtime.needs_present = false;
        return Ok(end_frame(runtime, again, false));
    }
    let follow = scheduler::present_followup(after);
    runtime.needs_present = follow.needs_present;
    if follow.clear_surface {
        runtime.surfaces.clear();
        runtime.installed = None;
    }
    if matches!(after, AfterPresent::Shown { .. } | AfterPresent::Stop) {
        if matches!(after, AfterPresent::Shown { .. }) {
            runtime.busy.arm();
        }
        if runtime.present_failures >= present::PRESENT_BACKOFF_AFTER {
            tracing::info!(target: "impasto::render::present", "present recovered");
        }
        runtime.present_failures = 0;
        runtime.device_recoveries = 0;
        runtime.present_retry_at = None;
        if let Some(tree) = runtime.latest.clone() {
            write_inspect(
                runtime,
                &tree,
                &timing,
                &phase,
                issue_list(layout_report.as_deref().unwrap_or("[]")),
            );
        }
    }
    let mut again = follow.vsync == VsyncOp::Request;
    if after == AfterPresent::Retry {
        runtime.present_failures = runtime.present_failures.saturating_add(1);
        if let Some(delay) = present::present_retry_delay_ms(runtime.present_failures) {
            if runtime.present_failures == present::PRESENT_BACKOFF_AFTER {
                tracing::warn!(
                    target: "impasto::render::present",
                    failures = runtime.present_failures,
                    retry_in_ms = delay,
                    "present failed; backing off"
                );
                if let Some(hub) = runtime.hub.as_ref() {
                    hub.error(
                        "engine",
                        "present failed; backing off",
                        None,
                        Some(runtime.frame_number),
                    );
                }
            }
            runtime.present_retry_at = Some(crate::net::ms_i64(crate::net::mono_ms()) + delay);
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
        pixels: target.pixels,
    };
    let opened = {
        let mut engine = runtime.engine.borrow_mut();
        runtime
            .surfaces
            .ready(target, &mut engine, runtime.open_surface.as_mut())
    };
    match opened {
        Ready::Opened { replaced, .. } => {
            runtime.services.borrow().surface_attached(kept.id);
            // The first real window can select a different backend from the
            // initial graphics context. Publish its factory only after it opens.
            *lock(&runtime.upload) = runtime.engine.borrow().texture_factory();
            if matches!(place, OpenPlace::Command) {
                if runtime.input.set_metrics(metrics) {
                    arm(runtime);
                }
                arm(runtime);
            } else {
                runtime.input.set_metrics(metrics);
            }
            tracing::info!(
                target: "impasto::render::surface",
                surface = kept.id.0,
                "surface opened"
            );
            Ok(OpenedSurface::Ready { replaced })
        }
        Ready::Failed(
            engine::Status::NotReady | engine::Status::SurfaceLost | engine::Status::AcquireMiss,
        ) => {
            let retry = runtime.surfaces.fail_attempt(kept, attempts);
            if matches!(retry, OpenRetry::GaveUp) {
                tracing::error!(
                    target: "impasto::render::surface",
                    "surface open failed; stopping retry"
                );
            } else {
                tracing::warn!(
                    target: "impasto::render::surface",
                    attempts,
                    "surface open not ready"
                );
                arm(runtime);
            }
            Ok(OpenedSurface::Waiting)
        }
        Ready::Failed(status) => {
            let error = BootError::Engine(status);
            tracing::error!(target: "impasto::render::surface", %status, "surface open failed");
            Err(error)
        }
    }
}

fn images_ready(runtime: &mut Runtime, census: Option<&uicore::ImageCensus>) -> bool {
    let requirements = &runtime.image_requirements;
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
        if source.starts_with(protocol::EXTERNAL_TEXTURE_PREFIX) {
            continue;
        }
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
                    tracing::error!(target: "impasto::render::image", "{message}");
                }
            }
            continue;
        }
        if let Some(request) = runtime.images.request(source, bucket)
            && runtime.image_tx.send(request).is_err()
        {
            tracing::error!(target: "impasto::render::image", "image decoder stopped");
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
            session.next_wake_ms().map(crate::net::ms_i64)
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
    } else if runtime.scheduler.token_armed() {
        // A posted vsync still owns this token. Cancelling it would freeze input.
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

fn phase_name(phase: uicore::Phase) -> &'static str {
    match phase {
        uicore::Phase::Down => "down",
        uicore::Phase::Move => "move",
        uicore::Phase::Up => "up",
        uicore::Phase::Cancel => "cancel",
        uicore::Phase::Wheel => "wheel",
        uicore::Phase::Zoom => "zoom",
    }
}

fn input_kind(event: &session::InputEvent) -> &'static str {
    match event {
        session::InputEvent::Key(_) | session::InputEvent::Back => "key",
        session::InputEvent::Text(_) | session::InputEvent::Paste { .. } => "text",
        session::InputEvent::Composing(_) | session::InputEvent::DismissIme(_) => "ime",
        session::InputEvent::Copy
        | session::InputEvent::Cut
        | session::InputEvent::SelectAll
        | session::InputEvent::Edit(_)
        | session::InputEvent::Replace { .. }
        | session::InputEvent::ReplaceIme { .. } => "edit",
    }
}

fn note_step(runtime: &mut Runtime, step: FrameStep) {
    let gpu = runtime.gpu;
    if runtime.last_step == Some((step, gpu)) {
        return;
    }
    runtime.last_step = Some((step, gpu));
    tracing::debug!(target: "impasto::render", ?step, ?gpu, "frame step");
}

fn frame_listeners(
    devtools: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) -> Vec<Box<dyn crate::timing::FrameListener>> {
    let mut listeners: Vec<Box<dyn crate::timing::FrameListener>> = vec![
        Box::new(crate::timing::BudgetEdge::default()),
        Box::new(crate::timing::WorkEdge::default()),
    ];
    if let Some(devtools) = devtools {
        listeners.push(Box::new(crate::timing::DevtoolFrames::new(devtools)));
    }
    listeners
}

/// Releases GPU caches and refuses acquire until a later submit rebuilds them.
fn flush_gpu(runtime: &mut Runtime, reason: &'static str) {
    tracing::info!(target: "impasto::render::gpu", reason, "gpu flush");
    runtime.engine.borrow_mut().trim();
    runtime.installed = None;
    runtime.gpu = Gpu::Flush;
    runtime.needs_present = false;
    runtime.present_failures = 0;
    runtime.present_retry_at = None;
    note_wake(runtime, None);
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

/// Registers queued faces before `submit`. A failure logs the family and status
/// only, then the frame continues with the faces already on the engine.
fn install_fonts(engine: &mut engine::Engine, fonts: Vec<(String, Vec<u8>)>) {
    for (family, bytes) in fonts {
        if let Err(status) = engine.register_font(&bytes, &family) {
            tracing::warn!(target: "impasto::shell", %family, %status, "font registration failed");
        } else {
            tracing::info!(target: "impasto::shell", %family, "font registered");
        }
    }
}

fn prepare_engine(
    services: &dyn Services,
    family: &str,
    open: impl FnOnce() -> Result<engine::Engine, engine::Status>,
) -> Result<Rc<RefCell<engine::Engine>>, BootError> {
    let mut engine = match open() {
        Ok(engine) => engine,
        Err(status) => {
            let error = BootError::Engine(status);
            tracing::error!(target: "impasto::shell", %status, "engine was not created");
            return Err(error);
        }
    };
    let Some(font) = services.font(family) else {
        let error = BootError::Font(family.to_owned());
        tracing::error!(target: "impasto::shell", %family, "font was not provided");
        return Err(error);
    };
    if let Err(status) = engine.register_font(font, family) {
        let error = BootError::Engine(status);
        tracing::error!(target: "impasto::shell", %family, %status, "font registration failed");
        return Err(error);
    }
    tracing::info!(target: "impasto::shell", %family, "font registered");
    for (name, bytes) in services.extra_fonts() {
        if let Err(status) = engine.register_font(&bytes, &name) {
            tracing::warn!(target: "impasto::shell", %name, %status, "font registration failed");
        } else {
            tracing::info!(target: "impasto::shell", %name, "font registered");
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
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use session::{FrameOutput, FrameTime, IoEvent, Metrics};

    use super::install_fonts;
    use super::{
        Acks, DropWait, ExitGuard, Installed, Runtime, fault_followup, lock, on_command, on_frame,
        start_async,
    };
    use crate::command::{Command, FrameToken, SurfaceId, SurfaceTarget};
    use crate::images::{ImageHandle, ImageReady};
    use crate::scheduler::FrameScheduler;
    use crate::scheduler::Gpu;
    use crate::surface::{SURFACE_OPEN_ATTEMPTS, SurfaceHost};
    use crate::{ClipboardHost, NativePtr, Presented, Surface, Vsync};

    #[test]
    fn a_rejected_font_is_logged_without_its_bytes() {
        let secret = b"SECRET-FONT-BYTES-DO-NOT-LOG";
        let lines = Arc::new(Mutex::new(Vec::new()));
        let capture = Capture(Arc::clone(&lines));
        tracing::subscriber::with_default(capture, || match engine::Engine::create_metal() {
            Ok(mut engine) => {
                install_fonts(&mut engine, vec![("Custom".to_owned(), secret.to_vec())]);
            }
            Err(engine::Status::NotLinked) => {
                tracing::warn!(
                    target: "impasto::shell",
                    family = "Custom",
                    status = "not linked",
                    "font registration failed"
                );
            }
            Err(status) => panic!("{status}"),
        });
        let lines = lines.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!lines.is_empty(), "a bad face must be reported");
        let message = &lines[0];
        assert!(message.contains("Custom"), "{message}");
        assert!(message.contains("font"), "{message}");
        assert!(!message.contains("SECRET-FONT-BYTES"), "{message}");
    }

    struct NullServices;

    impl crate::Services for NullServices {
        fn font(&self, _: &str) -> Option<&[u8]> {
            None
        }

        fn apply_ime(&self, _: &session::ImeUpdate) {}
    }

    struct NullSurface;

    impl Surface for NullSurface {
        fn present(
            &mut self,
            _: &mut engine::Engine,
            _: engine::PixelSize,
        ) -> Result<Presented, engine::Status> {
            Ok(Presented::Continue)
        }
    }

    struct CountSurface {
        presents: Arc<AtomicU32>,
        pixels: Arc<Mutex<engine::PixelSize>>,
    }

    impl Surface for CountSurface {
        fn present(
            &mut self,
            _: &mut engine::Engine,
            pixels: engine::PixelSize,
        ) -> Result<Presented, engine::Status> {
            self.presents.fetch_add(1, Ordering::Relaxed);
            *lock(&self.pixels) = pixels;
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

        fn pointer(&mut self, _: uicore::PointerEvent) {}

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
                session: Some(Box::new(MockSession {
                    log: Rc::clone(&log),
                })),
                net_tx: mpsc::sync_channel(1).0,
                registry: Arc::new(session::ModuleRegistry::new(Vec::new())),
                latest: None,
                image_requirements: Vec::new(),
                external_textures: Default::default(),
                external_gpu: Default::default(),
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
                upload: Arc::new(Mutex::new(None)),
                frame_number: 0,
                semantics: None,
                semantics_enabled: true,
                autofill_enabled: false,
                platform_views: Arc::default(),
                platform_views_surface: None,
                platform_views_dirty: false,
                wake_tx,
                dump_list: false,
                present_failures: 0,
                device_recoveries: 0,
                present_retry_at: None,
                gpu: Gpu::Live,
                last_step: None,
                listeners: Vec::new(),
                inspect: None,
                hub: None,
                inspect_dir: None,
                inspect_written: false,
                busy: Arc::new(crate::watchdog::Busy::new()),
                diag_armed: false,
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
            assert!(!self.command(Command::SurfaceReady(SurfaceTarget::new(
                id,
                NativePtr(std::ptr::null_mut()),
                Metrics::new(360.0, 640.0, 2.0),
            ))));
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
    fn launch_url_delivers_and_arms_an_idle_surface() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        harness.runtime.scheduler.cancel();
        assert!(!harness.runtime.scheduler.token_armed());

        assert!(!harness.command(Command::LaunchUrl("https://app.example/item".into())));
        assert!(harness.runtime.scheduler.token_armed());
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
    fn wake_does_not_cancel_an_armed_vsync() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let token = harness
            .vsync
            .last_token()
            .expect("SurfaceReady arms one vsync token");
        let cancels_before = harness.vsync.cancel_count();
        assert!(!harness.command(Command::Wake));
        assert_eq!(harness.frame_count(), 0);
        assert_eq!(harness.vsync.cancel_count(), cancels_before);
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(harness.frame_count(), 1);
    }

    #[test]
    fn wake_without_an_armed_token_runs_a_frame() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let token = harness
            .vsync
            .last_token()
            .expect("SurfaceReady arms one vsync token");
        assert!(!harness.command(Command::Frame { token, time: now() }));
        // SurfaceReady arms twice; the leftover pending token is not a timer wake.
        harness.runtime.scheduler.cancel();
        harness.runtime.vsync.cancel();
        let frames_before = harness.frame_count();
        assert!(!harness.command(Command::Wake));
        assert!(
            harness.frame_count() > frames_before,
            "an idle surface must still run a timer wake"
        );
    }

    #[test]
    fn resumed_replaces_a_stuck_armed_token() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let stuck = harness
            .vsync
            .last_token()
            .expect("SurfaceReady arms one vsync token");
        assert!(
            harness.runtime.scheduler.request().is_none(),
            "an armed token must block the next request"
        );
        let cancels_before = harness.vsync.cancel_count();
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Resumed)));
        assert!(harness.vsync.cancel_count() > cancels_before);
        let next = harness
            .vsync
            .last_token()
            .expect("Resumed arms a replacement token");
        assert_ne!(next, stuck);
        assert!(harness.runtime.scheduler.accept(next));
    }

    #[test]
    fn resumed_clears_present_backoff_and_rearms() {
        let mut harness = TestHarness::new();
        let target = SurfaceTarget::new(
            SurfaceId::fresh(),
            NativePtr(std::ptr::null_mut()),
            Metrics::new(360.0, 640.0, 2.0),
        );
        harness
            .runtime
            .surfaces
            .fail_attempt(target, SURFACE_OPEN_ATTEMPTS - 1);
        assert!(harness.runtime.surfaces.take_attempt().is_none());
        harness.runtime.needs_present = false;
        harness.runtime.present_failures = 3;
        harness.runtime.present_retry_at = Some(1);
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Resumed)));
        assert!(harness.runtime.needs_present);
        assert_eq!(harness.runtime.present_failures, 0);
        assert!(harness.runtime.present_retry_at.is_none());
        assert!(harness.runtime.surfaces.take_attempt().is_some());
    }

    #[test]
    fn visible_inactive_window_recovers_frames_after_being_hidden() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let before = harness.vsync.last_token();
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Hidden)));
        assert!(harness.runtime.input.is_paused());

        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Inactive)));
        assert!(!harness.runtime.input.is_paused());
        let token = harness
            .vsync
            .last_token()
            .expect("visible window arms vsync");
        assert_ne!(Some(token), before);
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(harness.frame_count(), 1);
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
                gpu: None,
            }));
        assert!(harness.runtime.images.contains("asset:logo", 1));
        harness.runtime.installed = Some(Installed { scale: 2.0 });
        let tokens_before = harness.vsync.token_count();
        assert!(!harness.command(Command::MemoryWarning));
        assert!(harness.runtime.installed.is_none());
        assert!(!harness.runtime.images.contains("asset:logo", 1));
        assert_eq!(harness.runtime.gpu, Gpu::Flush);
        assert!(harness.vsync.token_count() > tokens_before);
    }

    #[test]
    fn hidden_flushes_the_gpu_and_does_not_acquire() {
        let presents = Arc::new(AtomicU32::new(0));
        let mut harness = TestHarness::new();
        let counted = Arc::clone(&presents);
        harness.runtime.open_surface = Box::new(move |_, _| {
            let presents = Arc::clone(&counted);
            Ok(Box::new(CountSurface {
                presents,
                pixels: Arc::new(Mutex::new(engine::PixelSize::default())),
            }))
        });
        harness.open_dummy();
        harness.runtime.latest = Some(Arc::new(protocol::LayerTree::empty()));
        harness.runtime.installed = Some(Installed { scale: 2.0 });
        harness.runtime.needs_present = true;
        assert!(!harness.command(Command::Lifecycle(session::Lifecycle::Hidden)));
        assert_eq!(harness.runtime.gpu, Gpu::Flush);
        assert!(harness.runtime.installed.is_none());
        assert!(harness.runtime.input.is_paused());
        assert!(!on_frame(&mut harness.runtime, now(), &harness.acks).unwrap());
        assert_eq!(presents.load(Ordering::Relaxed), 0);
        harness.runtime.gpu = Gpu::Dead;
        harness.runtime.input.resume();
        assert!(!on_frame(&mut harness.runtime, now(), &harness.acks).unwrap());
        assert_eq!(presents.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn present_passes_the_pixels_the_host_posted() {
        let seen = Arc::new(Mutex::new(engine::PixelSize::default()));
        let mut harness = TestHarness::new();
        let seen_open = Arc::clone(&seen);
        harness.runtime.open_surface = Box::new(move |_, _| {
            Ok(Box::new(CountSurface {
                presents: Arc::new(AtomicU32::new(0)),
                pixels: Arc::clone(&seen_open),
            }))
        });
        harness.open_dummy();
        harness.runtime.latest = Some(Arc::new(protocol::LayerTree::empty()));
        harness.runtime.installed = Some(Installed { scale: 2.0 });
        harness.runtime.point_scale = 2.0;
        harness.runtime.needs_present = true;
        assert!(!on_frame(&mut harness.runtime, now(), &harness.acks).unwrap());
        assert_eq!(*lock(&seen), engine::PixelSize::new(720, 1280));
    }

    #[test]
    fn device_loss_keeps_the_latest_layers_and_requests_a_fresh_submission() {
        struct LostDevice(Arc<AtomicU32>);

        impl Surface for LostDevice {
            fn present(
                &mut self,
                _: &mut engine::Engine,
                _: engine::PixelSize,
            ) -> Result<Presented, engine::Status> {
                Err(engine::Status::DeviceLost)
            }

            fn recover_device(&mut self, _: &mut engine::Engine) -> Result<(), engine::Status> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }

        let recoveries = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&recoveries);
        let mut harness = TestHarness::new();
        harness.runtime.open_surface =
            Box::new(move |_, _| Ok(Box::new(LostDevice(Arc::clone(&counted)))));
        harness.open_dummy();
        let latest = Arc::new(protocol::LayerTree::empty());
        harness.runtime.latest = Some(Arc::clone(&latest));
        harness.runtime.installed = Some(Installed { scale: 2.0 });
        harness.runtime.point_scale = 2.0;
        harness.runtime.needs_present = true;

        let token = harness.vsync.last_token().expect("initial vsync");
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);
        assert!(harness.runtime.installed.is_none());
        assert_eq!(harness.runtime.gpu, Gpu::Flush);
        assert!(
            harness
                .runtime
                .latest
                .as_ref()
                .is_some_and(|tree| Arc::ptr_eq(tree, &latest))
        );
        assert_ne!(harness.vsync.last_token(), Some(token));
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
    fn faults_arm_one_extra_frame_then_stop_when_held() {
        let listener = session::ScriptFault {
            source: "listener",
            message: "boom".into(),
            stack: "at f".into(),
        };
        let frame = session::ScriptFault {
            source: "frame",
            message: "interrupted (frame)".into(),
            stack: String::new(),
        };
        let first = fault_followup(false, std::slice::from_ref(&listener));
        assert!(first.arm_extra);
        assert!(!first.stop_again);
        assert!(first.diag_armed);
        let second = fault_followup(true, &[frame]);
        assert!(!second.arm_extra);
        assert!(second.stop_again);
        let quiet = fault_followup(true, &[]);
        assert!(!quiet.diag_armed);
    }

    #[test]
    fn a_panicking_frame_stays_on_the_render_thread() {
        struct PanicSession;
        impl session::Session for PanicSession {
            fn metrics(&mut self, _: &Metrics) {}
            fn pointer(&mut self, _: uicore::PointerEvent) {}
            fn frame(&mut self, _: FrameTime) -> FrameOutput {
                panic!("js host");
            }
        }
        let mut harness = TestHarness::new();
        harness.open_dummy();
        harness.runtime.session = Some(Box::new(PanicSession));
        let token = harness
            .vsync
            .last_token()
            .expect("SurfaceReady arms one vsync token");
        assert!(!harness.command(Command::Frame { token, time: now() }));
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
        assert!(harness.acks.arm(id));
        assert!(!harness.acks.poll(id));
        harness.runtime.installed = Some(Installed { scale: 1.0 });
        let cancels_before = harness.vsync.cancel_count();
        assert!(!harness.command(Command::SurfaceDestroyed { id }));
        assert!(harness.runtime.installed.is_none());
        assert!(!harness.runtime.surfaces.is_open());
        assert!(harness.acks.poll(id));
        assert!(harness.vsync.cancel_count() > cancels_before);
        assert!(
            harness.runtime.scheduler.request().is_some(),
            "cancel must drop the armed token so the next request is accepted"
        );
    }

    #[test]
    fn renderer_exit_completes_existing_and_future_surface_teardown() {
        let acks = Arc::new(Acks::new());
        let pending = SurfaceId::fresh();
        let late = SurfaceId::fresh();
        assert!(acks.arm(pending));
        assert!(!acks.poll(pending));
        let (done, exited) = mpsc::channel();
        ExitGuard {
            done: Some(done),
            acks: Arc::clone(&acks),
            inbox: Arc::new(crate::queue::Inbox::new()),
        }
        .finish(Ok(()));

        assert!(matches!(exited.try_recv(), Ok(Ok(()))));
        assert!(!acks.arm(late));
        assert!(acks.poll(pending));
        assert!(acks.poll(late));
        DropWait { acks }.wait(pending);
    }

    #[test]
    fn closing_during_preparation_reports_cancelled_once() {
        use crate::BootError;
        use std::time::Duration;

        let (entered, preparing) = mpsc::channel();
        let (proceed, released) = mpsc::channel();
        let (report, ready) = mpsc::channel();
        let timeout = Duration::from_secs(5);
        let link = start_async(
            move || {
                entered.send(()).unwrap();
                released.recv_timeout(timeout).unwrap();
                Err(BootError::Platform("injected preparation failure".into()))
            },
            move |status| report.send(status).unwrap(),
        )
        .expect("spawn render thread");

        preparing
            .recv_timeout(timeout)
            .expect("preparation started");
        assert!(matches!(ready.try_recv(), Err(mpsc::TryRecvError::Empty)));
        link.request_shutdown();
        proceed.send(()).unwrap();

        assert!(matches!(
            ready.recv_timeout(timeout),
            Ok(Err(BootError::Cancelled))
        ));
        assert!(matches!(
            link.shutdown_and_join(),
            Err(BootError::Cancelled)
        ));
        assert!(matches!(
            ready.recv_timeout(timeout),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn buffer_resize_repaints_even_when_logical_metrics_are_unchanged() {
        let mut harness = TestHarness::new();
        let id = harness.open_dummy();
        // An idle frame has already delivered these logical metrics.
        let metrics = Metrics::new(360.0, 640.0, 2.0);
        assert_eq!(harness.runtime.input.begin_frame().metrics, Some(metrics));
        harness.runtime.needs_present = false;
        harness.runtime.scheduler.cancel();

        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics,
            pixels: engine::PixelSize::new(800, 1280),
        }));
        assert!(harness.runtime.needs_present);
        assert!(harness.runtime.scheduler.token_armed());
        assert!(harness.runtime.input.begin_frame().metrics.is_none());
    }

    #[test]
    fn remaining_command_arms_are_reachable() {
        let mut harness = TestHarness::new();
        harness.session.borrow_mut().wants = true;
        let id = harness.open_dummy();
        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics: Metrics::new(400.0, 800.0, 2.0),
            pixels: engine::PixelSize::new(800, 1600),
        }));
        assert!(!harness.command(Command::Pointer(uicore::PointerEvent::new(
            uicore::Phase::Move,
            1.0,
            1.0,
            0.0,
        ))));
        assert!(!harness.command(Command::Input(session::InputEvent::Back)));
        assert!(!harness.command(Command::ImeDismissed));
        assert!(!harness.command(Command::Metrics(Metrics::new(360.0, 640.0, 3.0))));
        assert!(!harness.command(Command::Invalidate));
        assert!(harness.runtime.needs_present);
        assert!(!harness.command(Command::Settings(session::PlatformSettings::default())));
        assert!(!harness.command(Command::WindowFocus(true)));
        assert!(!harness.command(Command::LaunchUrl("https://app.example/item".into())));
        assert!(
            !harness.command(Command::SemanticsAction(uicore::SemanticsAction::Activate(
                uicore::SemanticsId(1)
            )))
        );
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
        assert!(!harness.command(Command::SurfaceReady(SurfaceTarget::new(
            replaced,
            NativePtr(std::ptr::null_mut()),
            Metrics::new(360.0, 640.0, 2.0),
        ))));
        let frames_before = harness.frame_count();
        assert!(!harness.command(Command::Wake));
        assert_eq!(
            harness.frame_count(),
            frames_before,
            "Wake must not steal the vsync token SurfaceReady just armed"
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

    struct Capture(Arc<Mutex<Vec<String>>>);

    struct MessageVisit<'a>(&'a mut String);

    impl tracing::field::Visit for MessageVisit<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            if field.name() == "message" {
                *self.0 = value.to_owned();
            } else {
                self.0.push(' ');
                self.0.push_str(field.name());
                self.0.push('=');
                self.0.push_str(value);
            }
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                *self.0 = format!("{value:?}");
            } else {
                self.0.push(' ');
                self.0.push_str(field.name());
                self.0.push('=');
                self.0.push_str(&format!("{value:?}"));
            }
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = String::new();
            event.record(&mut MessageVisit(&mut message));
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(message);
        }

        fn enter(&self, _: &tracing::span::Id) {}

        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[test]
    fn editor_copy_without_clipboard_logs_not_linked() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut harness = TestHarness::new();
        harness.open_dummy();
        harness.session.borrow_mut().clipboard = Some("copied".into());
        let token = match harness.vsync.last_token() {
            Some(token) => token,
            None => panic!("SurfaceReady arms one vsync token"),
        };
        let capture = Capture(Arc::clone(&lines));
        tracing::subscriber::with_default(capture, || {
            assert!(!harness.command(Command::Frame { token, time: now() }));
        });
        let lines = lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            lines.iter().any(|message| message.contains("not linked")),
            "{lines:?}"
        );
    }

    #[test]
    fn not_ready_surface_waits_and_a_gave_up_change_rearms() {
        let mut harness = TestHarness::new();
        harness.runtime.open_surface = Box::new(|_, _| Err(engine::Status::NotReady));
        let id = SurfaceId::fresh();
        assert!(!harness.command(Command::SurfaceReady(SurfaceTarget::new(
            id,
            NativePtr(std::ptr::null_mut()),
            Metrics::new(360.0, 640.0, 2.0),
        ))));
        assert!(!harness.runtime.surfaces.is_open());
        harness.runtime.surfaces.fail_attempt(
            SurfaceTarget::new(
                id,
                NativePtr(std::ptr::null_mut()),
                Metrics::new(360.0, 640.0, 2.0),
            ),
            SURFACE_OPEN_ATTEMPTS - 1,
        );
        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics: Metrics::new(400.0, 800.0, 2.0),
            pixels: engine::PixelSize::new(800, 1600),
        }));
        assert!(harness.runtime.surfaces.take_attempt().is_some());
    }
}
