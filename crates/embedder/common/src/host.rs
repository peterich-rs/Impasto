//! Dispatches platform commands and coordinates the UI and raster owners.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engine::session::FrameTime;

use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::diagnostics::TelemetryHub;
use crate::diagnostics::telemetry::{
    IoSpan, ScheduleTiming, TelemetryProfile, mono_ns, mono_ns_at,
};
use crate::error::BootError;
use crate::images::{self, ImageCache, ImageDecoder, ImageError, ImageReady, ImageRequest};
use crate::input::{FrameInput, Input};
use crate::queue::Inbox;
use crate::scheduler::{FrameScheduler, Gpu};

use crate::surface::{OpenRetry, Ready, SurfaceHost};
use crate::task_runner::{RunnerSlot, WorkerTaskRunner};
use crate::timing::FrameTiming;
use crate::{
    Boot, Launch, Services, TaskHandle, TaskRunner, TaskRunners, ThreadTaskRunner, ThreadTopology,
};

mod cache_cmds;
mod frame_cmds;
mod raster_host;
use raster_host::{FrameSubmission, RasterCompletion, RasterHost, RasterPort, raster_frame};
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
    shell: Arc<Mutex<Option<Arc<Shell>>>>,
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
        if !self.acks.arm(id) {
            return;
        }
        let shell = lock(&self.shell).clone();
        if let Some(shell) = shell
            && shell.ui.runner().runs_on_current_thread()
            && shell.retire_surface_on_raster(id)
        {
            return;
        }
        self.inbox.post(Command::SurfaceDestroyed { id });
        self.drain_inline();
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
        self.drain_inline();
        let shell = lock(&self.shell).clone();
        if let Some(shell) = shell
            && shell.ui.runner().runs_on_current_thread()
            && lock(&shell.exit).is_some()
        {
            return Err(BootError::Platform(
                "cannot join a busy UI queue from the UI thread".into(),
            ));
        }
        self.join()
    }

    fn drain_inline(&self) {
        let shell = lock(&self.shell).clone();
        if let Some(shell) = shell
            && shell.ui.runner().runs_on_current_thread()
        {
            shell.drain(usize::MAX);
        }
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
    runtime: &mut UiHost,
    tree: &engine::scene::LayerTree,
    timing: &FrameTiming,
    phases: &engine::session::FramePhases,
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
    let (link, startup, guard) = new_link(Box::new(move |result| {
        let _ = ready_tx.send(result);
    }));
    launch_boot(
        Ok(boot),
        None,
        &link.shell,
        startup,
        guard,
        Box::new(|_| {}),
    );
    ready_rx.recv().map_err(|_| BootError::Panicked)??;
    Ok(link)
}

/// Starts a host with an explicit topology, including a manual event loop.
pub fn start_on(boot: Boot, runners: TaskRunners) -> Result<Link, BootError> {
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (link, startup, guard) = new_link(Box::new(move |result| {
        let _ = ready_tx.send(result);
    }));
    launch_boot(
        Ok(boot),
        Some(runners),
        &link.shell,
        startup,
        guard,
        Box::new(|_| {}),
    );
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

fn new_link(on_started: StartupCallback) -> (Link, Startup, ExitGuard) {
    let (done, receiver) = mpsc::channel();
    let inbox = Arc::new(Inbox::new());
    let acks = Arc::new(Acks::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let link = Link {
        inbox: Arc::clone(&inbox),
        done: Mutex::new(Some(receiver)),
        acks: Arc::clone(&acks),
        cancelled: Arc::clone(&cancelled),
        shell: Arc::default(),
    };
    (
        link,
        Startup {
            cancelled,
            notify: Some(on_started),
        },
        ExitGuard {
            done: Some(done),
            acks,
            inbox,
        },
    )
}

/// Prepares boot inputs on an I/O worker and creates the session on its UI runner.
pub fn start_async(
    prepare: impl FnOnce() -> Result<Boot, BootError> + Send + 'static,
    on_started: impl FnOnce(Result<(), BootError>) + Send + 'static,
) -> Result<Link, BootError> {
    start_async_with_exit(prepare, on_started, |_| {})
}

/// Notifies the caller after thread-affine resources and the inbox are closed.
pub fn start_async_with_exit(
    prepare: impl FnOnce() -> Result<Boot, BootError> + Send + 'static,
    on_started: impl FnOnce(Result<(), BootError>) + Send + 'static,
    on_exit: impl FnOnce(Result<(), BootError>) + Send + 'static,
) -> Result<Link, BootError> {
    let worker = WorkerTaskRunner::new(crate::rt::handle().map_err(BootError::Platform)?);
    let (link, startup, guard) = new_link(Box::new(on_started));
    let slot = Arc::clone(&link.shell);
    worker.post(Box::new(move || {
        let boot = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            startup.checkpoint()?;
            let boot = prepare();
            startup.checkpoint()?;
            boot
        }))
        .unwrap_or(Err(BootError::Panicked));
        launch_boot(boot, None, &slot, startup, guard, Box::new(on_exit));
    }));
    Ok(link)
}

fn launch_boot(
    boot: Result<Boot, BootError>,
    runners: Option<TaskRunners>,
    slot: &Mutex<Option<Arc<Shell>>>,
    mut startup: Startup,
    guard: ExitGuard,
    on_exit: StartupCallback,
) {
    let initialized = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        boot.and_then(|boot| {
            let runners = match runners {
                Some(runners) => runners,
                None => task_runners(&boot)?,
            };
            let fonts = collect_fonts(&boot)?;
            Ok((boot, runners, fonts))
        })
    }))
    .unwrap_or(Err(BootError::Panicked));
    let (boot, runners, fonts) = match initialized {
        Ok(pair) => pair,
        Err(error) => {
            guard.finish(Err(error.clone()));
            startup.report(Err(error.clone()));
            on_exit(Err(error));
            return;
        }
    };
    let ui = Arc::clone(&runners.ui);
    let shell = Arc::new(Shell {
        inbox: Arc::clone(&guard.inbox),
        acks: Arc::clone(&guard.acks),
        ui: RunnerSlot::new(Arc::clone(&ui)),
        remote_raster: Mutex::default(),
        queued: AtomicBool::new(false),
        exit: Mutex::new(Some((guard, on_exit))),
    });
    *lock(slot) = Some(Arc::clone(&shell));
    let initialize = move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            startup.checkpoint()?;
            let mut runtime = run_boot(boot, fonts, &shell.inbox, runners, &startup)?;
            startup.checkpoint()?;
            publish_back_ownership(&mut runtime);
            if let RasterPort::Remote(slot) = &runtime.graphics {
                *lock(&shell.remote_raster) = Some(Arc::clone(slot));
            }
            shell.ui.install(runtime);
            Ok(())
        }))
        .unwrap_or(Err(BootError::Panicked));
        if let Err(error) = result {
            startup.report(Err(error.clone()));
            shell.finish(Err(error));
            return;
        }
        let weak = Arc::downgrade(&shell);
        shell.inbox.bind(Arc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.schedule();
            }
        }));
        startup.report(Ok(()));
    };
    if ui.runs_on_current_thread() {
        initialize();
    } else {
        ui.post(Box::new(initialize));
    }
}

fn task_runners(boot: &Boot) -> Result<TaskRunners, BootError> {
    let thread = |name| {
        ThreadTaskRunner::start(name)
            .map(|runner| runner as Arc<dyn TaskRunner>)
            .map_err(|error| BootError::Platform(error.to_string()))
    };
    let io = Arc::new(WorkerTaskRunner::new(
        crate::rt::handle().map_err(BootError::Platform)?,
    ));
    let ui = if boot.thread_topology == ThreadTopology::PlatformUi {
        boot.platform_runner.clone().ok_or_else(|| {
            BootError::Platform("platform UI topology requires a native main-loop runner".into())
        })?
    } else {
        thread(if boot.thread_topology == ThreadTopology::SharedRender {
            "render"
        } else {
            "ui"
        })?
    };
    let raster = if boot.thread_topology == ThreadTopology::SharedRender {
        Arc::clone(&ui)
    } else {
        thread("raster")?
    };
    Ok(TaskRunners {
        platform: boot
            .platform_runner
            .clone()
            .unwrap_or_else(|| Arc::clone(&ui)),
        ui,
        raster,
        io,
    })
}

struct Shell {
    inbox: Arc<Inbox>,
    acks: Arc<Acks>,
    ui: RunnerSlot<UiHost>,
    remote_raster: Mutex<Option<Arc<RunnerSlot<RasterHost>>>>,
    queued: AtomicBool,
    exit: Mutex<Option<(ExitGuard, StartupCallback)>>,
}

impl Shell {
    fn schedule(self: &Arc<Self>) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            let shell = Arc::clone(self);
            self.ui.runner().post(Box::new(move || {
                shell.queued.store(false, Ordering::Release);
                shell.drain(32);
            }));
        }
    }

    fn drain(self: &Arc<Self>, limit: usize) {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.ui.with(|runtime| {
                for _ in 0..limit {
                    if let Some(completion) = self.inbox.take_completion() {
                        if completion(runtime)? {
                            return Ok(true);
                        }
                        continue;
                    }
                    let Some(command) = self.inbox.pop() else { return Ok(false) };
                    match on_command(runtime, command, &self.acks) {
                        Ok(true) => return Ok(true),
                        Ok(false) => publish_back_ownership(runtime),
                        Err(error) => {
                            tracing::error!(target: "impasto::shell", %error, "UI host stopped");
                            runtime.services.borrow().request_exit();
                            return Err(error);
                        }
                    }
                }
                Ok(false)
            })
            }))
            .unwrap_or(Some(Err(BootError::Panicked)));
        match result {
            Some(Ok(true)) => self.finish(Ok(())),
            Some(Err(error)) => self.finish(Err(error)),
            Some(Ok(false)) if self.inbox.has_pending() => self.schedule(),
            _ => {}
        }
    }

    // UIKit/Android can synchronously retire a window from within a platform
    // callback while UiHost is already borrowed. Raster only publishes back
    // to the UI queue, so this acknowledgement never needs the main loop.
    fn retire_surface_on_raster(&self, id: SurfaceId) -> bool {
        let Some(slot) = lock(&self.remote_raster).clone() else {
            return false;
        };
        self.inbox.discard_surface(id);
        let inbox = Arc::clone(&self.inbox);
        let acks = Arc::clone(&self.acks);
        Arc::clone(slot.runner()).post(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                slot.with(|graphics| surface_cmds::retire(graphics, id))
            }));
            inbox.complete(Box::new(move |runtime| match result {
                Ok(dropped) => {
                    surface_cmds::did_retire(runtime, id, dropped.unwrap_or(false));
                    Ok(false)
                }
                Err(_) => Err(BootError::Panicked),
            }));
            acks.signal(id);
        }))
    }

    fn finish(&self, result: Result<(), BootError>) {
        let retired = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.ui.clear()));
        let result = if retired.is_err() {
            Err(BootError::Panicked)
        } else {
            result
        };
        lock(&self.remote_raster).take();
        let exit = lock(&self.exit).take();
        if let Some((guard, on_exit)) = exit {
            guard.finish(result.clone());
            on_exit(result);
        }
    }
}

fn run_boot(
    boot: Boot,
    fonts: StartupFonts,
    inbox: &Arc<Inbox>,
    runners: TaskRunners,
    startup: &Startup,
) -> Result<UiHost, BootError> {
    crate::diagnostics::telemetry::install_system_trace();
    let trace_path =
        std::env::var_os("IMPASTO_TRACE").filter(|path| !path.is_empty() && path != "1");
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    let remote = crate::diagnostics::remote::Config::configured(boot.log_dir.as_deref());
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    let remote_enabled = remote.is_some();
    #[cfg(not(any(debug_assertions, feature = "devtools-server")))]
    let remote_enabled = false;
    let profile = TelemetryProfile::configured(boot.devtools.is_some());
    let profile = if remote_enabled && std::env::var_os("IMPASTO_TELEMETRY").is_none() {
        profile.max(TelemetryProfile::Counters)
    } else {
        profile
    };
    let telemetry = boot.devtools.clone().or_else(|| {
        (profile != TelemetryProfile::Off || trace_path.is_some() || remote_enabled)
            .then(|| Arc::new(TelemetryHub::new()))
    });
    if let Some(hub) = &telemetry {
        hub.set_profile(profile);
    }
    let enable_gpu_timing = trace_path.is_some()
        || remote_enabled
        || profile == TelemetryProfile::GpuAndTimeline
        || (boot.devtools.is_some() && cfg!(debug_assertions) && profile != TelemetryProfile::Off);
    if telemetry.is_some() || boot.inspect.is_some() || boot.inspect_dir.is_some() {
        mono_ns();
    }
    let capture_runner = Arc::clone(&runners.io);
    crate::log_sub::install(boot.log_dir.as_deref(), boot.devtools.clone());
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    let remote = match (remote, telemetry.clone()) {
        (Some(config), Some(hub)) => match crate::diagnostics::remote::Server::start(config, hub) {
            Ok(server) => Some(server),
            Err(error) => {
                tracing::warn!(target: "impasto::devtools", %error, "DevTools listener unavailable");
                None
            }
        },
        _ => None,
    };
    let upload = Arc::new(Mutex::new(None));
    if let Some(hub) = boot.devtools.as_ref() {
        install_panic_hook(Arc::clone(hub));
    }
    let http = boot
        .http
        .clone()
        .map(|inner| crate::diagnostics::wrap_if_devtools(inner, boot.devtools.clone()));
    let load_image: ImageLoader = Arc::new({
        let decoder = Arc::clone(&boot.decoder);
        let inbox = Arc::clone(inbox);
        let http = http.clone();
        let upload = Arc::clone(&upload);
        let io = Arc::clone(&runners.io);
        let telemetry = telemetry.clone();
        move |request| {
            let decoder = Arc::clone(&decoder);
            let inbox = Arc::clone(&inbox);
            let http = http.clone();
            let upload = Arc::clone(&upload);
            let telemetry = telemetry.clone();
            io.post(Box::new(move || {
                inbox.post(Command::ImageReady(decode_one(
                    decoder.as_ref(),
                    http.as_deref(),
                    &request,
                    &upload,
                    telemetry.as_deref(),
                )));
            }))
        }
    });
    let runtime = open_ui(
        boot,
        fonts,
        inbox,
        startup,
        runners,
        RenderLinks {
            load_image,
            http,
            upload,
            telemetry: telemetry.clone(),
            enable_gpu_timing,
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            remote,
        },
    )?;
    if let (Some(path), Some(hub)) = (trace_path, telemetry)
        && let Some(id) = hub.start_trace(path.into(), true)
    {
        capture_runner.post_delayed(Duration::from_secs(5), Box::new(move || {
            match hub.finish_trace(id) {
                Ok(path) => tracing::info!(target: "impasto::trace", path = %path.display(), "trace exported"),
                Err(error) => tracing::warn!(target: "impasto::trace", %error, "trace export failed"),
            }
        }));
    }
    Ok(runtime)
}

type ImageLoader = Arc<dyn Fn(ImageRequest) -> bool + Send + Sync>;

fn decode_one(
    decoder: &dyn ImageDecoder,
    http: Option<&dyn crate::http::HttpClient>,
    request: &ImageRequest,
    upload: &Mutex<Option<raster::TextureFactory>>,
    telemetry: Option<&TelemetryHub>,
) -> ImageReady {
    let span_start = || telemetry.filter(|hub| hub.recording()).map(|_| mono_ns());
    let note = |name, start| {
        if let (Some(hub), Some(start_ns)) = (telemetry, start) {
            hub.note_io(IoSpan {
                name,
                start_ns,
                end_ns: mono_ns(),
            });
        }
    };
    let mut decode_start = span_start();
    let result = if request.source.starts_with("https://") {
        let body = crate::http::fetch_image_body(http, &request.source);
        note("Image fetch", decode_start);
        decode_start = span_start();
        match body {
            Ok(bytes) => decoder.decode_bytes(&request.source, &bytes),
            Err(error) => Err(ImageError::Failed(error)),
        }
    } else if request.source.starts_with("http://") {
        Err(ImageError::Failed("image URL must be https".into()))
    } else {
        decoder.decode_at(&request.source, request.scale_bucket)
    };
    note("Image decode", decode_start);
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
            let upload_start = span_start();
            let factory = lock(upload).clone();
            images::attach_upload(&mut handle, factory.as_ref());
            note("Image upload", upload_start);
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

pub(crate) struct UiHost {
    shaper: Rc<RefCell<raster::ParleyShaper>>,
    graphics: RasterPort,
    active_surface: Option<SurfaceId>,
    try_surface: bool,
    revision: u64,
    frames: FramePipeline,
    inbox: Arc<Inbox>,
    services: Rc<RefCell<Box<dyn Services>>>,
    session: Option<Box<dyn engine::session::Session>>,
    registry: Arc<engine::session::ModuleRegistry>,
    // WindowSession keeps a weak subscription to this poster.
    _window: Arc<Poster>,
    latest: Option<Arc<engine::scene::LayerTree>>,
    image_requirements: Arc<[(Arc<str>, bool)]>,
    input: Input,
    scheduler: FrameScheduler,
    vsync: Arc<dyn crate::Vsync>,
    point_scale: f32,
    frame_number: u64,
    topology: ThreadTopology,
    telemetry: Option<Arc<TelemetryHub>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    _remote: Option<crate::diagnostics::remote::Server>,
    gpu_poll: bool,
    semantics: Option<engine::semantics::SemanticsUpdate>,
    semantics_enabled: bool,
    autofill_enabled: bool,
    platform_views: Arc<[engine::tree::PlatformViewPlacement]>,
    published_views: Option<(SurfaceId, Arc<[engine::tree::PlatformViewPlacement]>)>,

    wake: WakeTimer,
    dump_list: bool,
    present_retry_at: Option<i64>,
    listeners: Vec<Box<dyn crate::timing::FrameListener>>,
    inspect: Option<std::path::PathBuf>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
    inspect_dir: Option<std::path::PathBuf>,
    inspect_written: bool,
    busy: Arc<crate::watchdog::Busy>,
    /// One extra frame was already armed for the latest script fault.
    diag_armed: bool,
}

impl UiHost {
    fn update_raster<R: Send + 'static>(
        &mut self,
        work: impl FnOnce(&mut RasterHost) -> R + Send + 'static,
        apply: impl FnOnce(&mut Self, R) + Send + 'static,
    ) {
        match &mut self.graphics {
            RasterPort::Local(graphics) => {
                let result = work(graphics);
                apply(self, result);
            }
            RasterPort::Remote(_) => {
                let inbox = Arc::clone(&self.inbox);
                self.graphics.post(move |graphics| {
                    let result = work(graphics);
                    inbox.complete(Box::new(move |ui| {
                        apply(ui, result);
                        Ok(false)
                    }));
                });
            }
        }
    }
}

impl Drop for UiHost {
    fn drop(&mut self) {
        self.vsync.cancel();
        self.session.take();
        self.graphics.retire();
    }
}

#[derive(Default)]
struct FramePipeline {
    in_flight: usize,
    waiting: bool,
}

impl FramePipeline {
    const DEPTH: usize = 2;

    fn has_room(&mut self) -> bool {
        let room = self.in_flight < Self::DEPTH;
        self.waiting |= !room;
        room
    }

    fn submit(&mut self) {
        self.in_flight += 1;
    }

    fn complete(&mut self) -> bool {
        self.in_flight -= 1;
        std::mem::take(&mut self.waiting)
    }
}

struct RenderLinks {
    load_image: ImageLoader,
    telemetry: Option<Arc<TelemetryHub>>,
    enable_gpu_timing: bool,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    remote: Option<crate::diagnostics::remote::Server>,
    http: Option<Arc<dyn crate::http::HttpClient>>,
    upload: Arc<Mutex<Option<raster::TextureFactory>>>,
}

struct StartupFonts {
    family: String,
    primary: Arc<[u8]>,
    extras: Vec<(String, Arc<[u8]>)>,
}

fn collect_fonts(boot: &Boot) -> Result<StartupFonts, BootError> {
    let family = boot.font_family.clone();
    let font = boot
        .services
        .font(&family)
        .ok_or_else(|| BootError::Font(family.clone()))?;
    let extras = boot
        .services
        .extra_fonts()
        .into_iter()
        .map(|(name, bytes)| (name, Arc::from(bytes)))
        .collect();
    Ok(StartupFonts {
        family,
        primary: Arc::from(font),
        extras,
    })
}

fn open_ui(
    boot: Boot,
    fonts: StartupFonts,
    inbox: &Arc<Inbox>,
    startup: &Startup,
    runners: TaskRunners,
    links: RenderLinks,
) -> Result<UiHost, BootError> {
    startup.checkpoint()?;
    let services = Rc::new(RefCell::new(boot.services));
    let StartupFonts {
        family,
        primary,
        extras,
    } = fonts;
    let split = runners.ui.queue_id() != runners.raster.queue_id();
    let library = raster::FontLibrary::new();
    let mut shaper = raster::ParleyShaper::new(library.clone());
    if !shaper.register_shared(&family, primary) {
        return Err(BootError::Font(family));
    }
    for (family, bytes) in extras {
        shaper.register_shared(&family, bytes);
    }
    let shaper = Rc::new(RefCell::new(shaper));
    let raster_busy = Arc::new(crate::watchdog::Busy::new());
    if split {
        crate::watchdog::spawn(
            "raster",
            Arc::downgrade(&raster_busy),
            boot.devtools.clone(),
        );
    }
    let raster_clock = Arc::clone(&runners.raster);
    let open = boot.open_raster;
    let open_surface = boot.open_surface;
    let raster_telemetry = links.telemetry.clone();
    let create = move || {
        let mut raster = open(library).map_err(BootError::Engine)?;
        if links.enable_gpu_timing {
            raster.enable_gpu_timing();
        }
        Ok::<_, BootError>(RasterHost {
            raster: Rc::new(RefCell::new(raster)),
            telemetry: raster_telemetry,
            surfaces: SurfaceHost::new(),
            open_surface,
            external_textures: Default::default(),
            external_gpu: Default::default(),
            installed: None,
            needs_present: false,
            images: ImageCache::new(),
            load_image: links.load_image,
            upload: links.upload,
            present_failures: 0,
            device_recoveries: 0,
            frame_retry_at: None,
            clock: raster_clock,
            gpu: Gpu::Live,
            last_step: None,
            busy: raster_busy,
        })
    };
    let graphics = if split {
        let slot = Arc::new(RunnerSlot::new(Arc::clone(&runners.raster)));
        let target = Arc::clone(&slot);
        let (ready, response) = mpsc::sync_channel(1);
        runners.raster.post(Box::new(move || {
            let result = create().map(|raster| target.install(raster));
            let _ = ready.send(result);
        }));
        response.recv().map_err(|_| BootError::Panicked)??;
        RasterPort::Remote(slot)
    } else {
        RasterPort::Local(Box::new(create()?))
    };
    startup.checkpoint()?;
    let Launch::Session(open) = boot.launch;
    let parts = crate::SessionOpen {
        shaper: Box::new(Rc::clone(&shaper)),
        log: console_log(),
    };
    let app_state = engine::session::shared("active".to_owned());
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
    let has_http = boot.modules.iter().any(|m| m.specifier == "@impasto/http");
    let has_ws = boot
        .modules
        .iter()
        .any(|m| m.specifier == "@impasto/websocket");
    if !has_http {
        modules.push(crate::modules::http(links.http));
    }
    if !has_ws {
        modules.push(crate::modules::websocket(
            services.borrow().websocket_transport(),
            boot.devtools.clone(),
        ));
    }
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
    let registry = Arc::new(engine::session::ModuleRegistry::new(modules));
    let host = engine::session::ModuleConfig {
        registry: Arc::clone(&registry),
        exec: Arc::new(crate::RtExec {
            io: Arc::clone(&runners.io),
            main: boot
                .platform_runner
                .as_ref()
                .map(|_| Arc::clone(&runners.platform)),
        }),
        settle: crate::settle_factory(Arc::clone(inbox)),
        post_delivery: Some(Arc::new({
            let inbox = Arc::clone(inbox);
            move |delivery| inbox.post(Command::JsIo(delivery))
        })),
        transport_clock: Some(Arc::new({
            let clock = Arc::clone(&runners.ui);
            move || crate::timing::ms_i64(clock.monotonic_ms())
        })),
        script_execution: if boot.thread_topology == ThreadTopology::PlatformUi {
            engine::session::ScriptExecution::PlatformUi
        } else {
            engine::session::ScriptExecution::DedicatedUi
        },
        app_state,
        launch_url,
    };
    startup.checkpoint()?;
    let session = open(parts, host);
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
    let frame_clock = Arc::clone(&runners.ui);
    boot.vsync.bind(Arc::new(move |token, mut time| {
        time.mono_ms = frame_clock.monotonic_ms();
        inbox_frames.post(Command::Frame { token, time });
    }));
    let busy = Arc::new(crate::watchdog::Busy::new());
    crate::watchdog::spawn("ui", Arc::downgrade(&busy), boot.devtools.clone());
    Ok(UiHost {
        graphics,
        shaper,
        active_surface: None,
        try_surface: false,
        revision: 0,
        frames: FramePipeline::default(),
        inbox: Arc::clone(inbox),
        services,
        session,
        registry,
        _window: window,
        latest: None,
        image_requirements: Arc::default(),
        input: Input::new(),
        scheduler: FrameScheduler::new(),
        vsync: boot.vsync,
        point_scale: 1.0,
        frame_number: 0,
        topology: boot.thread_topology,
        telemetry: links.telemetry,
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        _remote: links.remote,
        gpu_poll: false,
        semantics: None,
        semantics_enabled: true,
        autofill_enabled: false,
        platform_views: Arc::default(),
        published_views: None,
        wake: WakeTimer::new(runners.ui, Arc::clone(inbox)),
        dump_list: boot.dump_list,
        present_retry_at: None,
        listeners: frame_listeners(),
        inspect: boot.inspect,
        hub: boot.devtools.clone(),
        inspect_dir: boot.inspect_dir,
        inspect_written: false,
        busy,
        diag_armed: false,
    })
}

fn update_source(runtime: &mut UiHost, revision: &str, source: &str) {
    let Some(session) = runtime.session.as_mut() else {
        return;
    };
    let restoration = runtime.services.borrow().restoration();
    let staged = restoration.as_ref().map(crate::Restoration::stage_update);
    let now = runtime.wake.runner.monotonic_ms();
    let result = session.update_source(
        source,
        Box::new(Rc::clone(&runtime.shaper)),
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
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.graphics.post(|graphics| {
            graphics.raster.borrow_mut().invalidate_pictures();
            graphics.installed = None;
        });
        runtime.semantics = None;
        runtime.diag_armed = false;
        let services = runtime.services.borrow();
        services.apply_ime(&engine::session::ImeUpdate::Hide);
        services.semantics(&[]);
        runtime.platform_views = Arc::default();
        runtime.published_views = None;
        if let Some(surface) = runtime.active_surface {
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

fn publish_back_ownership(runtime: &mut UiHost) {
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

fn fault_followup(diag_armed: bool, faults: &[engine::session::ScriptFault]) -> FaultPlan {
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

fn on_command(runtime: &mut UiHost, command: Command, acks: &Acks) -> Result<bool, BootError> {
    runtime.busy.enter(crate::command::command_tag(&command));
    let result = dispatch_command(runtime, command, acks);
    runtime.busy.leave();
    result
}

fn dispatch_command(
    runtime: &mut UiHost,
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
        | Command::Io(_)) => session_cmds::handle(runtime, command),
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

fn fold_semantics(
    snapshot: &mut Option<engine::semantics::SemanticsUpdate>,
    patch: &engine::semantics::SemanticsPatch,
) {
    let update = snapshot.get_or_insert_with(|| engine::semantics::SemanticsUpdate {
        root: patch.root,
        nodes: Vec::new(),
    });
    update.root = patch.root;
    for op in &patch.ops {
        match op {
            engine::semantics::SemanticsOp::Upsert(node) => {
                if let Some(found) = update.nodes.iter_mut().find(|item| item.id == node.id) {
                    *found = node.clone();
                } else {
                    update.nodes.push(node.clone());
                }
            }
            engine::semantics::SemanticsOp::Remove(id) => {
                update.nodes.retain(|node| node.id != *id)
            }
        }
    }
    if let Some(order) = &patch.order {
        let ranks: HashMap<_, _> = order.iter().copied().zip(0..).collect();
        update
            .nodes
            .sort_by_key(|node| ranks.get(&node.id).copied().unwrap_or(usize::MAX));
    }
}

fn layer_tree_line(tree: &engine::scene::LayerTree) -> String {
    let pictures: Vec<String> = tree
        .pictures
        .iter()
        .map(|picture| {
            format!(
                "{}:gen{}:{}cmds",
                picture.id.0,
                picture.generation,
                picture.commands().len()
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

fn ms_to_ns(ms: f64) -> u64 {
    Duration::try_from_secs_f64(ms / 1_000.0)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .unwrap_or_default()
}

fn on_frame(
    runtime: &mut UiHost,
    time: FrameTime,
    from_vsync: bool,
    acks: &Acks,
) -> Result<bool, BootError> {
    use engine::session::trace::Section;

    let _frame_trace = Section::begin(c"Impasto frame");
    let frame_started = Instant::now();
    let counters = runtime
        .telemetry
        .as_ref()
        .is_some_and(|hub| hub.collects_frames());
    let inspect = runtime.inspect.is_some() || runtime.inspect_dir.is_some();
    let schedule = (counters || inspect).then(|| {
        let ui_start_ns = mono_ns_at(frame_started);
        // FrameTime::mono_ms was stamped when the callback posted to the
        // inbox. Use that runner's elapsed time so custom epochs also work.
        let vsync_ns = from_vsync.then(|| {
            let queued_ms = (runtime.wake.runner.monotonic_ms() - time.mono_ms).max(0.0);
            ui_start_ns.saturating_sub(ms_to_ns(queued_ms))
        });
        ScheduleTiming {
            vsync_ns,
            ui_start_ns,
            topology: runtime.topology,
            budget_ns: ms_to_ns(time.interval_ms),
            ..ScheduleTiming::default()
        }
    });
    if !runtime.frames.has_room() {
        return Ok(end_frame(runtime, false, false));
    }
    let attempt = if runtime.try_surface || matches!(runtime.graphics, RasterPort::Local(_)) {
        let attempt = runtime
            .graphics
            .with(|graphics| graphics.surfaces.take_attempt())?;
        runtime.try_surface = attempt.is_some();
        attempt
    } else {
        None
    };
    if let Some((target, attempts)) = attempt {
        match open_surface_target(runtime, target, attempts, OpenPlace::Frame)? {
            OpenedSurface::Ready { replaced } => {
                if let Some(old) = replaced {
                    acks.signal(old);
                }
            }
            OpenedSurface::Waiting => return Ok(end_frame(runtime, false, false)),
        }
    }
    if runtime.active_surface.is_none() {
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
    static NEXT_FRAME: AtomicU64 = AtomicU64::new(1);
    runtime.frame_number = NEXT_FRAME.fetch_add(1, Ordering::Relaxed);
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
    let mut census = None;
    let mut layers_changed = false;
    let mut placements_changed = false;
    let mut phases = None;
    let mut runtime_sample = None;
    let mut layout_report = None;
    if let Some(session) = runtime.session.as_mut() {
        session.set_counters(counters);
        let framed = {
            let _trace = Section::begin(c"Impasto session");
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.frame(time)))
        };
        match framed {
            Ok(mut output) => {
                if counters && runtime.frame_number.is_multiple_of(30) {
                    runtime_sample = session.runtime_sample();
                }
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
                    runtime.image_requirements = tree.image_requirements().into();
                    runtime.latest = Some(tree);
                    layers_changed = true;
                }
                if let Some(text) = output.clipboard.take()
                    && let Err(message) = runtime.registry.call(
                        "Clipboard",
                        "setString",
                        &[engine::session::PluginValue::Str(text)],
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
                    placements_changed = true;
                }
                reloads = std::mem::take(&mut output.reloads);
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
    let submission = FrameSubmission {
        revision: runtime.revision,
        number: runtime.frame_number,
        time,
        frame_started,
        schedule: schedule.map(|schedule| ScheduleTiming {
            ui_done_ns: mono_ns(),
            ..schedule
        }),
        telemetry: runtime.telemetry.clone().filter(|_| counters),
        runtime_sample,
        session_ms: elapsed_ms(session_started),
        gesture_ms,
        phases: phases.unwrap_or_default(),
        layout_report,
        measure: runtime.shaper.borrow_mut().take_shape_stats(),
        scale: runtime.point_scale,
        paused: frame.paused,
        again,
        tree: runtime.latest.clone(),
        requirements: Arc::clone(&runtime.image_requirements),
        platform_views: Arc::clone(&runtime.platform_views),
        placements_changed,
        layers_changed,
        reloads,
        census,
        hub: runtime.hub.clone(),
    };
    match &mut runtime.graphics {
        RasterPort::Local(graphics) => {
            let completion = raster_frame(graphics, submission, false, || {
                if let Some(token) = runtime.scheduler.prepare_next() {
                    runtime.vsync.request_frame(token);
                }
            });
            finish_raster_frame(runtime, completion, false)
        }
        RasterPort::Remote(slot) => {
            let slot = Arc::clone(slot);
            let inbox = Arc::clone(&runtime.inbox);
            runtime.frames.submit();
            let run = Arc::clone(slot.runner()).post(Box::new(move || {
                let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    slot.with(|graphics| raster_frame(graphics, submission, true, || {}))
                        .unwrap_or(Err(BootError::Panicked))
                }))
                .unwrap_or(Err(BootError::Panicked));
                inbox.complete(Box::new(move |runtime| {
                    finish_raster_frame(runtime, completion, true)
                }));
            }));
            if !run {
                return Err(BootError::Panicked);
            }
            Ok(end_frame(
                runtime,
                again && runtime.frames.in_flight < FramePipeline::DEPTH,
                false,
            ))
        }
    }
}

fn finish_raster_frame(
    runtime: &mut UiHost,
    completion: Result<RasterCompletion, BootError>,
    split: bool,
) -> Result<bool, BootError> {
    let waiting = split && runtime.frames.complete();
    let RasterCompletion {
        submission,
        output,
        surface,
        retry_at,
    } = completion?;
    if output.gpu_pending {
        schedule_gpu_poll(runtime);
    }
    if let Some(timing) = output.timing {
        for listener in &mut runtime.listeners {
            listener.on_frame(&timing, &submission.phases);
        }
        if runtime.inspect_dir.is_none()
            && let Some(dir) = &runtime.inspect
            && let Err(error) =
                crate::timing::write_inspect(dir, &timing, submission.tree.as_deref())
        {
            tracing::warn!(target: "impasto::shell", %error, "inspect write failed");
        }
        if (submission.revision == runtime.revision || output.stop)
            && output.shown
            && let Some(tree) = submission.tree
        {
            write_inspect(
                runtime,
                &tree,
                &timing,
                &submission.phases,
                issue_list(submission.layout_report.as_deref().unwrap_or("[]")),
            );
        }
    }
    if output.stop {
        runtime.services.borrow().request_exit();
        return Ok(end_frame(runtime, false, true));
    }
    if submission.revision != runtime.revision {
        if waiting && !runtime.input.is_paused() {
            arm(runtime);
        }
        return Ok(false);
    }
    runtime.active_surface = surface;
    runtime.present_retry_at = retry_at;
    if output.shown {
        runtime.busy.arm();
        if let Some(surface) = surface {
            let views = submission.platform_views;
            let changed =
                runtime
                    .published_views
                    .as_ref()
                    .map_or(!views.is_empty(), |(id, prior)| {
                        (*id == surface && !Arc::ptr_eq(prior, &views))
                            || (*id != surface && !views.is_empty())
                    });
            if changed {
                runtime.services.borrow().platform_views(surface, &views);
                runtime.published_views = Some((surface, views));
            }
        }
    }
    let again = (waiting || output.again) && !runtime.input.is_paused();
    if split {
        refresh_wake(runtime);
        if again {
            arm(runtime);
        }
        return Ok(false);
    }
    Ok(end_frame(runtime, again, false))
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
    runtime: &mut UiHost,
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
    let opened = runtime.graphics.with(move |graphics| {
        let mut raster = graphics.raster.borrow_mut();
        let opened = graphics
            .surfaces
            .ready(target, &mut raster, graphics.open_surface.as_mut());
        if matches!(opened, Ready::Opened { .. }) {
            for frame in graphics.external_gpu.values_mut() {
                if frame.image.is_none() {
                    frame.image = (frame.import)(&raster);
                }
            }
            *lock(&graphics.upload) = raster.texture_factory();
            graphics.needs_present = true;
            graphics.installed = None;
        }
        opened
    })?;
    match opened {
        Ready::Opened { replaced, .. } => {
            runtime.services.borrow().surface_attached(kept.id);
            runtime.active_surface = Some(kept.id);
            runtime.try_surface = false;
            runtime.revision = runtime.revision.wrapping_add(1);
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
            raster::Status::NotReady | raster::Status::SurfaceLost | raster::Status::AcquireMiss,
        ) => {
            let retry = runtime
                .graphics
                .with(move |graphics| graphics.surfaces.fail_attempt(kept, attempts))?;
            runtime.try_surface = matches!(retry, OpenRetry::Again);
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

fn end_frame(runtime: &mut UiHost, again: bool, stop: bool) -> bool {
    if stop {
        runtime.scheduler.cancel();
        runtime.vsync.cancel();
        note_wake(runtime, None);
        return true;
    }
    refresh_wake(runtime);
    if let Some(token) = runtime.scheduler.finish(again) {
        runtime.vsync.request_frame(token);
    } else if runtime.scheduler.token_armed() {
        // A posted vsync still owns this token. Cancelling it would freeze input.
    } else {
        runtime.vsync.cancel();
    }
    false
}

fn refresh_wake(runtime: &mut UiHost) {
    let session_deadline = runtime.session.as_mut().and_then(|session| {
        if runtime.input.is_paused() {
            None
        } else {
            session.next_wake_ms().map(crate::timing::ms_i64)
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
}

struct WakeTimer {
    runner: Arc<dyn TaskRunner>,
    inbox: Arc<Inbox>,
    pending: RefCell<Option<(i64, TaskHandle)>>,
}

impl WakeTimer {
    fn new(runner: Arc<dyn TaskRunner>, inbox: Arc<Inbox>) -> Self {
        Self {
            runner,
            inbox,
            pending: RefCell::default(),
        }
    }

    fn set(&self, next: Option<i64>) {
        let mut pending = self.pending.borrow_mut();
        if let Some((deadline, handle)) = pending.take() {
            if next == Some(deadline) && handle.is_pending() {
                *pending = Some((deadline, handle));
                return;
            }
            handle.cancel();
        }
        if let Some(deadline) = next {
            let now = crate::timing::ms_i64(self.runner.monotonic_ms());
            let millis = u64::try_from(deadline.saturating_sub(now)).unwrap_or(0);
            let inbox = Arc::downgrade(&self.inbox);
            let handle = self.runner.post_delayed(
                Duration::from_millis(millis),
                Box::new(move || {
                    if let Some(inbox) = inbox.upgrade() {
                        inbox.post(Command::Wake);
                    }
                }),
            );
            *pending = Some((deadline, handle));
        }
    }
}

impl Drop for WakeTimer {
    fn drop(&mut self) {
        self.set(None);
    }
}

fn note_wake(runtime: &UiHost, deadline: Option<i64>) {
    // A due timer cannot drive a third frame. The next raster completion
    // releases the slot and reinstates the wake from the session deadline.
    runtime
        .wake
        .set(deadline.filter(|_| !runtime.frames.waiting));
}

fn arm(runtime: &mut UiHost) {
    if let Some(token) = runtime.scheduler.request() {
        runtime.vsync.request_frame(token);
    }
}

fn phase_name(phase: engine::input::Phase) -> &'static str {
    match phase {
        engine::input::Phase::Down => "down",
        engine::input::Phase::Move => "move",
        engine::input::Phase::Up => "up",
        engine::input::Phase::Cancel => "cancel",
        engine::input::Phase::Wheel => "wheel",
        engine::input::Phase::Zoom => "zoom",
    }
}

fn input_kind(event: &engine::session::InputEvent) -> &'static str {
    match event {
        engine::session::InputEvent::Key(_) | engine::session::InputEvent::Back => "key",
        engine::session::InputEvent::Text(_) | engine::session::InputEvent::Paste { .. } => "text",
        engine::session::InputEvent::Composing(_) | engine::session::InputEvent::DismissIme(_) => {
            "ime"
        }
        engine::session::InputEvent::Copy
        | engine::session::InputEvent::Cut
        | engine::session::InputEvent::SelectAll
        | engine::session::InputEvent::Edit(_)
        | engine::session::InputEvent::Replace { .. }
        | engine::session::InputEvent::ReplaceIme { .. } => "edit",
    }
}

fn frame_listeners() -> Vec<Box<dyn crate::timing::FrameListener>> {
    vec![
        Box::new(crate::timing::BudgetEdge::default()),
        Box::new(crate::timing::WorkEdge::default()),
    ]
}

fn schedule_gpu_poll(runtime: &mut UiHost) {
    let Some(hub) = runtime.telemetry.clone() else {
        return;
    };
    if runtime.gpu_poll {
        return;
    }
    runtime.gpu_poll = true;
    let inbox = Arc::downgrade(&runtime.inbox);
    runtime.wake.runner.post_delayed(
        Duration::from_millis(8),
        Box::new(move || {
            if let Some(inbox) = inbox.upgrade() {
                inbox.complete(Box::new(move |runtime| {
                    runtime.update_raster(
                        move |graphics| raster_host::drain_gpu(graphics, &hub),
                        |runtime, pending| {
                            runtime.gpu_poll = false;
                            if pending {
                                schedule_gpu_poll(runtime);
                            }
                        },
                    );
                    Ok(false)
                }));
            }
        }),
    );
}

/// Releases GPU caches and refuses acquire until a later submit rebuilds them.
fn flush_gpu(runtime: &mut UiHost, reason: &'static str) {
    tracing::info!(target: "impasto::render::gpu", reason, "gpu flush");
    runtime.revision = runtime.revision.wrapping_add(1);
    runtime.graphics.post(|graphics| {
        graphics.raster.borrow_mut().trim();
        graphics.installed = None;
        graphics.gpu = Gpu::Flush;
        graphics.needs_present = false;
        graphics.present_failures = 0;
        graphics.frame_retry_at = None;
    });
    runtime.present_retry_at = None;
    note_wake(runtime, None);
}

fn note_scale(runtime: &mut UiHost, next: f32) {
    if (next - runtime.point_scale).abs() > f32::EPSILON {
        runtime.graphics.post(|graphics| graphics.installed = None);
    }
    runtime.point_scale = next;
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
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

    use engine::session::{FrameOutput, FrameTime, IoEvent, Metrics};

    use super::{
        Acks, DropWait, ExitGuard, FramePipeline, Inbox, Installed, RasterHost, RasterPort, UiHost,
        WakeTimer, fault_followup, lock, on_command, on_frame, start_async,
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
        tracing::subscriber::with_default(capture, || {
            let mut shaper = raster::ParleyShaper::new(raster::FontLibrary::new());
            assert!(!shaper.register("Custom", secret));
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

        fn apply_ime(&self, _: &engine::session::ImeUpdate) {}
    }

    struct NullSurface;

    impl Surface for NullSurface {
        fn present(
            &mut self,
            _: &mut raster::Rasterizer,
            _: raster::PixelSize,
        ) -> Result<Presented, raster::Status> {
            Ok(Presented::Continue)
        }
    }

    struct CountSurface {
        presents: Arc<AtomicU32>,
        pixels: Arc<Mutex<raster::PixelSize>>,
    }

    impl Surface for CountSurface {
        fn present(
            &mut self,
            _: &mut raster::Rasterizer,
            pixels: raster::PixelSize,
        ) -> Result<Presented, raster::Status> {
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

    impl engine::session::Session for MockSession {
        fn metrics(&mut self, _: &Metrics) {}

        fn pointer(&mut self, _: engine::input::PointerEvent) {}

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
        runtime: UiHost,
        acks: Acks,
        vsync: Arc<RecVsync>,
        session: Rc<RefCell<SessionLog>>,
    }

    impl TestHarness {
        fn new() -> Self {
            let library = raster::FontLibrary::new();
            let raster = Rc::new(RefCell::new(
                raster::Rasterizer::create(raster::RasterOptions::default(), library.clone())
                    .unwrap_or_else(|error| panic!("{error}")),
            ));
            let vsync = RecVsync::new();
            let log = Rc::new(RefCell::new(SessionLog {
                io: Vec::new(),
                frames: Vec::new(),
                wants: false,
                launch_urls: Vec::new(),
                clipboard: None,
            }));
            let shaper = Rc::new(RefCell::new(raster::ParleyShaper::new(library)));
            let runtime = UiHost {
                shaper,
                graphics: RasterPort::Local(Box::new(RasterHost {
                    raster,
                    telemetry: None,
                    load_image: Arc::new(|_| true),
                    busy: Arc::new(crate::watchdog::Busy::new()),
                    frame_retry_at: None,
                    clock: Arc::new(crate::ManualTaskRunner::default()),
                    external_textures: Default::default(),
                    external_gpu: Default::default(),
                    surfaces: SurfaceHost::new(),
                    open_surface: Box::new(|_, _| Ok(Box::new(NullSurface))),
                    installed: None,
                    needs_present: false,
                    images: crate::images::ImageCache::new(),
                    upload: Arc::new(Mutex::new(None)),
                    present_failures: 0,
                    device_recoveries: 0,
                    gpu: Gpu::Live,
                    last_step: None,
                })),
                active_surface: None,
                try_surface: false,
                revision: 0,
                frames: FramePipeline::default(),
                inbox: Arc::new(Inbox::new()),
                services: Rc::new(RefCell::new(Box::new(NullServices))),
                session: Some(Box::new(MockSession {
                    log: Rc::clone(&log),
                })),
                registry: Arc::new(engine::session::ModuleRegistry::new(Vec::new())),
                _window: Arc::new(super::Poster {
                    inbox: Arc::new(Inbox::new()),
                }),
                latest: None,
                image_requirements: Arc::default(),
                input: crate::input::Input::new(),
                scheduler: FrameScheduler::new(),
                vsync: Arc::clone(&vsync) as Arc<dyn Vsync>,
                point_scale: 1.0,
                frame_number: 0,
                topology: crate::ThreadTopology::SharedRender,
                telemetry: None,
                #[cfg(any(debug_assertions, feature = "devtools-server"))]
                _remote: None,
                gpu_poll: false,
                semantics: None,
                semantics_enabled: true,
                autofill_enabled: false,
                platform_views: Arc::default(),
                published_views: None,
                wake: WakeTimer::new(
                    Arc::new(crate::ManualTaskRunner::default()),
                    Arc::new(Inbox::new()),
                ),
                dump_list: false,
                present_retry_at: None,
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
            assert!(self.runtime.graphics.local().surfaces.is_open());
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
    fn a_full_frame_pipeline_defers_ui_work_until_raster_releases_capacity() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        assert!(harness.runtime.frames.has_room());
        harness.runtime.frames.submit();
        assert!(harness.runtime.frames.has_room());
        harness.runtime.frames.submit();
        let token = harness
            .vsync
            .last_token()
            .expect("surface requests a frame");
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(harness.frame_count(), 0);
        assert!(harness.runtime.frames.complete());
        assert!(harness.runtime.frames.has_room());
        assert!(!harness.runtime.frames.complete());
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
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Paused)));
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
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Resumed)));
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
            .graphics
            .local()
            .surfaces
            .fail_attempt(target, SURFACE_OPEN_ATTEMPTS - 1);
        assert!(
            harness
                .runtime
                .graphics
                .local()
                .surfaces
                .take_attempt()
                .is_none()
        );
        harness.runtime.graphics.local().needs_present = false;
        harness.runtime.graphics.local().present_failures = 3;
        harness.runtime.present_retry_at = Some(1);
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Resumed)));
        assert!(harness.runtime.graphics.local().needs_present);
        assert_eq!(harness.runtime.graphics.local().present_failures, 0);
        assert!(harness.runtime.present_retry_at.is_none());
        assert!(
            harness
                .runtime
                .graphics
                .local()
                .surfaces
                .take_attempt()
                .is_some()
        );
    }

    #[test]
    fn visible_inactive_window_recovers_frames_after_being_hidden() {
        let mut harness = TestHarness::new();
        harness.open_dummy();
        let before = harness.vsync.last_token();
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Hidden)));
        assert!(harness.runtime.input.is_paused());

        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Inactive)));
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
            .graphics
            .local()
            .images
            .request("asset:logo", 1)
            .expect("pending decode");
        harness
            .runtime
            .graphics
            .local()
            .images
            .apply(ImageReady::Decoded(ImageHandle {
                source: Arc::from("asset:logo"),
                generation: request.generation,
                size: [1, 1],
                pixels: Arc::from([0u8, 0, 0, 0]),
                gpu: None,
            }));
        assert!(
            harness
                .runtime
                .graphics
                .local()
                .images
                .contains("asset:logo", 1)
        );
        harness.runtime.graphics.local().installed = Some(Installed { scale: 2.0 });
        let tokens_before = harness.vsync.token_count();
        assert!(!harness.command(Command::MemoryWarning));
        assert!(harness.runtime.graphics.local().installed.is_none());
        assert!(
            !harness
                .runtime
                .graphics
                .local()
                .images
                .contains("asset:logo", 1)
        );
        assert_eq!(harness.runtime.graphics.local().gpu, Gpu::Flush);
        assert!(harness.vsync.token_count() > tokens_before);
    }

    #[test]
    fn hidden_flushes_the_gpu_and_does_not_acquire() {
        let presents = Arc::new(AtomicU32::new(0));
        let mut harness = TestHarness::new();
        let counted = Arc::clone(&presents);
        harness.runtime.graphics.local().open_surface = Box::new(move |_, _| {
            let presents = Arc::clone(&counted);
            Ok(Box::new(CountSurface {
                presents,
                pixels: Arc::new(Mutex::new(raster::PixelSize::default())),
            }))
        });
        harness.open_dummy();
        harness.runtime.latest = Some(Arc::new(engine::scene::LayerTree::empty()));
        harness.runtime.graphics.local().installed = Some(Installed { scale: 2.0 });
        harness.runtime.graphics.local().needs_present = true;
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Hidden)));
        assert_eq!(harness.runtime.graphics.local().gpu, Gpu::Flush);
        assert!(harness.runtime.graphics.local().installed.is_none());
        assert!(harness.runtime.input.is_paused());
        assert!(!on_frame(&mut harness.runtime, now(), false, &harness.acks).unwrap());
        assert_eq!(presents.load(Ordering::Relaxed), 0);
        harness.runtime.graphics.local().gpu = Gpu::Dead;
        harness.runtime.input.resume();
        assert!(!on_frame(&mut harness.runtime, now(), false, &harness.acks).unwrap());
        assert_eq!(presents.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn present_passes_the_pixels_the_host_posted() {
        let seen = Arc::new(Mutex::new(raster::PixelSize::default()));
        let mut harness = TestHarness::new();
        let seen_open = Arc::clone(&seen);
        harness.runtime.graphics.local().open_surface = Box::new(move |_, _| {
            Ok(Box::new(CountSurface {
                presents: Arc::new(AtomicU32::new(0)),
                pixels: Arc::clone(&seen_open),
            }))
        });
        harness.open_dummy();
        harness.runtime.latest = Some(Arc::new(engine::scene::LayerTree::empty()));
        harness.runtime.graphics.local().installed = Some(Installed { scale: 2.0 });
        harness.runtime.point_scale = 2.0;
        harness.runtime.graphics.local().needs_present = true;
        assert!(!on_frame(&mut harness.runtime, now(), false, &harness.acks).unwrap());
        assert_eq!(*lock(&seen), raster::PixelSize::new(720, 1280));
    }

    #[test]
    fn device_loss_keeps_the_latest_layers_and_requests_a_fresh_submission() {
        struct LostDevice(Arc<AtomicU32>);

        impl Surface for LostDevice {
            fn present(
                &mut self,
                _: &mut raster::Rasterizer,
                _: raster::PixelSize,
            ) -> Result<Presented, raster::Status> {
                Err(raster::Status::DeviceLost)
            }

            fn recover_device(
                &mut self,
                _: &mut raster::Rasterizer,
                _: raster::PixelSize,
            ) -> Result<(), raster::Status> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }

        let recoveries = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&recoveries);
        let mut harness = TestHarness::new();
        harness.runtime.graphics.local().open_surface =
            Box::new(move |_, _| Ok(Box::new(LostDevice(Arc::clone(&counted)))));
        harness.open_dummy();
        let latest = Arc::new(engine::scene::LayerTree::empty());
        harness.runtime.latest = Some(Arc::clone(&latest));
        harness.runtime.graphics.local().installed = Some(Installed { scale: 2.0 });
        harness.runtime.point_scale = 2.0;
        harness.runtime.graphics.local().needs_present = true;

        let token = harness.vsync.last_token().expect("initial vsync");
        assert!(!harness.command(Command::Frame { token, time: now() }));
        assert_eq!(recoveries.load(Ordering::Relaxed), 1);
        assert!(harness.runtime.graphics.local().installed.is_none());
        assert_eq!(harness.runtime.graphics.local().gpu, Gpu::Flush);
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
        assert!(!harness.command(Command::Io(IoEvent::Plugin {
            token: 7,
            result: engine::session::PluginResult::Failed {
                message: "offline".into(),
            },
        })));
        assert!(
            matches!(
                harness.session.borrow().io.last(),
                Some(IoEvent::Plugin { token: 7, .. })
            ),
            "plugin failure completion must reach the session"
        );
        assert_eq!(harness.vsync.token_count(), tokens_before);
        harness.session.borrow_mut().wants = true;
        assert!(!harness.command(Command::Io(IoEvent::Plugin {
            token: 8,
            result: engine::session::PluginResult::Done(engine::session::PluginValue::Null),
        })));
        assert!(
            matches!(
                harness.session.borrow().io.last(),
                Some(IoEvent::Plugin { token: 8, .. })
            ),
            "plugin completion must reach the session"
        );
        assert!(harness.vsync.token_count() > tokens_before);
    }

    #[test]
    fn faults_arm_one_extra_frame_then_stop_when_held() {
        let listener = engine::session::ScriptFault {
            source: "listener",
            message: "boom".into(),
            stack: "at f".into(),
        };
        let frame = engine::session::ScriptFault {
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
        impl engine::session::Session for PanicSession {
            fn metrics(&mut self, _: &Metrics) {}
            fn pointer(&mut self, _: engine::input::PointerEvent) {}
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
        harness.runtime.graphics.local().installed = Some(Installed { scale: 1.0 });
        let cancels_before = harness.vsync.cancel_count();
        assert!(!harness.command(Command::SurfaceDestroyed { id }));
        assert!(harness.runtime.graphics.local().installed.is_none());
        assert!(!harness.runtime.graphics.local().surfaces.is_open());
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
            inbox: Arc::new(Inbox::new()),
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
        harness.runtime.graphics.local().needs_present = false;
        harness.runtime.scheduler.cancel();

        assert!(!harness.command(Command::SurfaceChanged {
            id,
            metrics,
            pixels: raster::PixelSize::new(800, 1280),
        }));
        assert!(harness.runtime.graphics.local().needs_present);
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
            pixels: raster::PixelSize::new(800, 1600),
        }));
        assert!(
            !harness.command(Command::Pointer(engine::input::PointerEvent::new(
                engine::input::Phase::Move,
                1.0,
                1.0,
                0.0,
            )))
        );
        assert!(!harness.command(Command::Input(engine::session::InputEvent::Back)));
        assert!(!harness.command(Command::ImeDismissed));
        assert!(!harness.command(Command::Metrics(Metrics::new(360.0, 640.0, 3.0))));
        assert!(!harness.command(Command::Invalidate));
        assert!(harness.runtime.graphics.local().needs_present);
        assert!(!harness.command(Command::Settings(
            engine::session::PlatformSettings::default()
        )));
        assert!(!harness.command(Command::WindowFocus(true)));
        assert!(!harness.command(Command::LaunchUrl("https://app.example/item".into())));
        assert!(!harness.command(Command::SemanticsAction(
            engine::semantics::SemanticsAction::Activate(engine::semantics::SemanticsId(1))
        )));
        assert!(!harness.command(Command::ReloadImage("asset:logo".into())));
        assert!(!harness.command(Command::ImageReady(ImageReady::Missing {
            source: Arc::from("asset:missing"),
            generation: 1,
        })));
        assert!(!harness.command(Command::Io(IoEvent::Plugin {
            token: 11,
            result: engine::session::PluginResult::Failed {
                message: "retry".into(),
            },
        })));
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Inactive)));
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
        assert!(!harness.command(Command::Lifecycle(engine::session::Lifecycle::Hidden)));
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
        harness.runtime.registry = Arc::new(engine::session::ModuleRegistry::new(vec![
            crate::clipboard(Arc::clone(&clip) as _),
        ]));
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
        harness.runtime.graphics.local().open_surface =
            Box::new(|_, _| Err(raster::Status::NotReady));
        let id = SurfaceId::fresh();
        assert!(!harness.command(Command::SurfaceReady(SurfaceTarget::new(
            id,
            NativePtr(std::ptr::null_mut()),
            Metrics::new(360.0, 640.0, 2.0),
        ))));
        assert!(!harness.runtime.graphics.local().surfaces.is_open());
        harness.runtime.graphics.local().surfaces.fail_attempt(
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
            pixels: raster::PixelSize::new(800, 1600),
        }));
        assert!(
            harness
                .runtime
                .graphics
                .local()
                .surfaces
                .take_attempt()
                .is_some()
        );
    }
}
