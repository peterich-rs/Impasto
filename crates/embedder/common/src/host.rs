//! Dispatches platform commands and coordinates the UI and raster owners.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use engine::session::FrameTime;

use crate::command::{Command, SurfaceId};
use crate::diagnostics::TelemetryHub;
use crate::diagnostics::telemetry::{TelemetryProfile, mono_ns};
use crate::error::BootError;
use crate::images::{self, ImageCache, ImageDecoder, ImageError, ImageReady, ImageRequest};
use crate::input::Input;
use crate::queue::Inbox;
use crate::scheduler::{FrameScheduler, Gpu};

use crate::surface::SurfaceHost;
use crate::task_runner::{RunnerSlot, WorkerTaskRunner};
use crate::timing::FrameTiming;
use crate::{
    Boot, Launch, Services, SwitchableTaskRunner, TaskHandle, TaskRunner, TaskRunners,
    ThreadTaskRunner,
};

mod cache_cmds;
mod compositor_animation;
mod frame_cmds;
mod raster_host;
use raster_host::{FrameSubmission, RasterCompletion, RasterHost, RasterPort, raster_frame};
mod session_cmds;
mod surface_cmds;
mod ui_frame;

use ui_frame::{on_frame, skipped_frame};

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

    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    pub(crate) fn post_devtools(&self, command: Command) -> bool {
        self.inbox.post_devtools(command)
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
        let _wait = tracing::info_span!(
            target: "impasto::render::surface",
            "surface_destroy_wait",
            surface = id.0,
            queue = "platform",
        )
        .entered();
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
        tracing::event!(name: "surface_retire_requested", target: "impasto::render::surface",
            tracing::Level::DEBUG, trace_event = true, surface = id.0);
        self.inbox.discard_surface(id);
        let shell = lock(&self.shell).clone();
        if let Some(shell) = shell
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
            && (shell.ui.runner().runs_on_current_thread()
                || shell
                    .handoff
                    .as_ref()
                    .is_some_and(|handoff| handoff.platform.runs_on_current_thread()))
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

pub(crate) fn host_os_version() -> String {
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
            let (runners, handoff) = match runners {
                Some(runners) => (runners, None),
                None => task_runners(&boot)?,
            };
            Ok((boot, runners, handoff))
        })
    }))
    .unwrap_or(Err(BootError::Panicked));
    let (boot, runners, handoff) = match initialized {
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
        handoff,
        queued: AtomicBool::new(false),
        draining: AtomicBool::new(false),
        exit: Mutex::new(Some((guard, on_exit))),
    });
    *lock(slot) = Some(Arc::clone(&shell));
    let initialize = move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            startup.checkpoint()?;
            let fonts = collect_fonts(&boot)?;
            let mut runtime = run_boot(boot, fonts, &shell, runners, &startup)?;
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

fn task_runners(boot: &Boot) -> Result<(TaskRunners, Option<PlatformHandoff>), BootError> {
    let thread = |name| {
        ThreadTaskRunner::start(name)
            .map(|runner| runner as Arc<dyn TaskRunner>)
            .map_err(|error| BootError::Platform(error.to_string()))
    };
    let io = Arc::new(WorkerTaskRunner::new(
        crate::rt::handle().map_err(BootError::Platform)?,
    ));
    let background = thread("ui-boot")?;
    let platform = boot
        .platform_runner
        .clone()
        .unwrap_or_else(|| Arc::clone(&background));
    let handoff = match (&boot.launch, &boot.platform_runner) {
        (Launch::Migrating(_), Some(platform)) => Some(PlatformHandoff {
            ui: Arc::new(SwitchableTaskRunner::new(Arc::clone(&background))),
            platform: Arc::clone(platform),
        }),
        _ => None,
    };
    let ui = handoff.as_ref().map_or(background, |handoff| {
        Arc::clone(&handoff.ui) as Arc<dyn TaskRunner>
    });
    Ok((
        TaskRunners {
            platform,
            ui,
            raster: thread("raster")?,
            io,
        },
        handoff,
    ))
}

struct PlatformHandoff {
    ui: Arc<SwitchableTaskRunner>,
    platform: Arc<dyn TaskRunner>,
}

struct MigratedUi(UiHost);

#[expect(
    unsafe_code,
    reason = "transfers the unique, quiescent platform UI host"
)]
// SAFETY: only Shell::migrate constructs this after extracting the only
// RunnerSlot owner and disarming the idle session. Launch::migrating guarantees
// that the session and its callbacks form a closed ownership graph with the
// host's shaper and VM. Platform services and plugin handles are Send, listeners
// are host-owned, and RasterPort is remote. The destination installs the value
// before resuming any UI callbacks.
unsafe impl Send for MigratedUi {}

impl MigratedUi {
    fn install(self, shell: &Shell, target: &dyn TaskRunner) {
        let mut ui = self.0;
        ui.queue_placement = "platform-ui";
        if let Some(session) = &mut ui.session {
            session.finish_platform_handoff();
        }
        shell.ui.install_migrated(target, ui);
    }
}

struct Shell {
    inbox: Arc<Inbox>,
    acks: Arc<Acks>,
    ui: RunnerSlot<UiHost>,
    remote_raster: Mutex<Option<Arc<RunnerSlot<RasterHost>>>>,
    handoff: Option<PlatformHandoff>,
    queued: AtomicBool,
    draining: AtomicBool,
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
        if self.draining.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.ui.with(|runtime| self.drain_commands(runtime, limit))
        }))
        .unwrap_or(Some(Err(BootError::Panicked)));
        self.draining.store(false, Ordering::Release);
        match result {
            Some(Ok(true)) => self.finish(Ok(())),
            Some(Err(error)) => self.finish(Err(error)),
            Some(Ok(false)) if !self.migrate() && self.inbox.has_pending() => self.schedule(),
            _ => {}
        }
    }

    fn drain_commands(
        &self,
        runtime: &mut UiHost,
        mut remaining: usize,
    ) -> Result<bool, BootError> {
        use crate::queue::Work;

        while remaining > 0 {
            match self.inbox.take_work(remaining) {
                Some(Work::Completions(batch)) => {
                    remaining -= batch.len();
                    for completion in batch {
                        if completion(runtime)? {
                            return Ok(true);
                        }
                    }
                }
                Some(Work::Command(command)) => {
                    remaining -= 1;
                    match on_command(runtime, command, &self.acks) {
                        Ok(true) => return Ok(true),
                        Ok(false) => publish_back_ownership(runtime),
                        Err(error) => {
                            tracing::error!(target: "impasto::shell", %error, "UI host stopped");
                            runtime.services.borrow().request_exit();
                            return Err(error);
                        }
                    }
                    if self.handoff.is_some() && runtime.handoff_ready {
                        break;
                    }
                }
                None => break,
            }
        }
        Ok(false)
    }

    fn migrate(self: &Arc<Self>) -> bool {
        let Some(handoff) = &self.handoff else {
            return false;
        };
        let prepared = self.ui.with(|ui| {
            if !std::mem::take(&mut ui.handoff_ready) || ui.queue_placement != "ui-boot" {
                return false;
            }
            ui.session
                .as_mut()
                .is_some_and(|session| session.prepare_platform_handoff())
        });
        if prepared != Some(true) {
            return false;
        }
        let Some(source) = handoff.ui.begin_migration() else {
            return false;
        };
        let Some(ui) = self.ui.extract(source.as_ref()) else {
            handoff.ui.complete_migration(source);
            self.finish(Err(BootError::Panicked));
            return true;
        };
        let payload = MigratedUi(ui);
        let shell = Arc::clone(self);
        let platform = Arc::clone(&handoff.platform);
        let target = Arc::clone(&platform);
        if !platform.post(Box::new(move || {
            let Some(handoff) = &shell.handoff else {
                return;
            };
            payload.install(&shell, target.as_ref());
            handoff.ui.complete_migration(target);
            shell.schedule();
        })) {
            handoff.ui.complete_migration(source);
            self.finish(Err(BootError::Cancelled));
        }
        true
    }

    // UIKit/Android can synchronously retire a window from within a platform
    // callback while UiHost is already borrowed. Raster only publishes back
    // to the UI queue, so this acknowledgement never needs the main loop.
    fn retire_surface_on_raster(&self, id: SurfaceId) -> bool {
        let Some(slot) = lock(&self.remote_raster).clone() else {
            return false;
        };
        let inbox = Arc::clone(&self.inbox);
        let acks = Arc::clone(&self.acks);
        if slot.runner().runs_on_current_thread() {
            let Some(dropped) = slot.with(|graphics| surface_cmds::retire(graphics, id)) else {
                return false;
            };
            inbox.complete(Box::new(move |runtime| {
                surface_cmds::did_retire(runtime, id, dropped);
                Ok(false)
            }));
            acks.signal(id);
            return true;
        }
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
    shell: &Arc<Shell>,
    runners: TaskRunners,
    startup: &Startup,
) -> Result<UiHost, BootError> {
    let inbox = &shell.inbox;
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
    let enable_gpu_timing =
        trace_path.is_some() || remote_enabled || profile == TelemetryProfile::GpuAndTimeline;
    if telemetry.is_some() || boot.inspect.is_some() || boot.inspect_dir.is_some() {
        mono_ns();
    }
    let capture_runner = Arc::clone(&runners.io);
    crate::log_sub::install(boot.log_dir.as_deref(), telemetry.clone());
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    let remote = match (remote, telemetry.clone()) {
        (Some(config), Some(hub)) => {
            match crate::diagnostics::remote::Server::start(
                config,
                hub,
                Poster {
                    inbox: Arc::clone(inbox),
                },
            ) {
                Ok(server) => Some(server),
                Err(error) => {
                    tracing::warn!(target: "impasto::devtools", %error, "DevTools listener unavailable");
                    None
                }
            }
        }
        _ => None,
    };
    let upload = Arc::new(Mutex::new(None));
    if let Some(hub) = telemetry.as_ref() {
        install_panic_hook(Arc::clone(hub));
    }
    let load_image: ImageLoader = Arc::new({
        let decoder = Arc::clone(&boot.decoder);
        let inbox = Arc::clone(inbox);
        let upload = Arc::clone(&upload);
        let io = Arc::clone(&runners.io);
        move |request| {
            let decoder = Arc::clone(&decoder);
            let inbox = Arc::clone(&inbox);
            let upload = Arc::clone(&upload);
            let request_id = request.generation;
            tracing::event!(name: "image_requested", target: "impasto::io", tracing::Level::DEBUG,
                trace_event = true, request_id);
            io.post(Box::new(move || {
                let _job = tracing::debug_span!(target: "impasto::io", "image_job", request_id, queue = "io").entered();
                let result = decode_one(decoder.as_ref(), &request, &upload);
                tracing::event!(name: "image_ready", target: "impasto::io", tracing::Level::DEBUG,
                    trace_event = true, request_id);
                inbox.post(Command::ImageReady(result));
            }))
        }
    });
    let startup_capture = trace_path.zip(telemetry.clone()).and_then(|(path, hub)| {
        let id = hub.start_trace(path.into(), true)?;
        Some((id, hub))
    });
    let runtime = open_ui(
        boot,
        fonts,
        shell,
        startup,
        runners,
        RenderLinks {
            load_image,
            upload,
            telemetry,
            enable_gpu_timing,
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            remote,
        },
    );
    if let Some((id, hub)) = startup_capture {
        if runtime.is_ok() {
            capture_runner.post_delayed(Duration::from_secs(5), Box::new(move || {
                match hub.finish_trace(id) {
                    Ok(path) => tracing::info!(target: "impasto::trace", path = %path.display(), "trace exported"),
                    Err(error) => tracing::warn!(target: "impasto::trace", %error, "trace export failed"),
                }
            }));
        } else {
            hub.cancel_capture(id);
        }
    }
    runtime
}

type ImageLoader = Arc<dyn Fn(ImageRequest) -> bool + Send + Sync>;

fn decode_one(
    decoder: &dyn ImageDecoder,
    request: &ImageRequest,
    upload: &Mutex<Option<raster::TextureFactory>>,
) -> ImageReady {
    let result = if request.source.starts_with("https://") {
        let body = {
            let _span =
                tracing::debug_span!(target: "impasto::io", "image_fetch", request_id = request.generation)
                    .entered();
            images::remote::fetch(&request.source)
        };
        let _span =
            tracing::debug_span!(target: "impasto::io", "image_decode", request_id = request.generation)
                .entered();
        match body {
            Ok(bytes) => decoder.decode_bytes_sized(&request.source, &bytes, request.target),
            Err(error) => Err(ImageError::Failed(error)),
        }
    } else if request.source.starts_with("http://") {
        Err(ImageError::Failed("image URL must be https".into()))
    } else {
        let _span =
            tracing::debug_span!(target: "impasto::io", "image_decode", request_id = request.generation)
                .entered();
        decoder.decode_sized(&request.source, request.scale_bucket, request.target)
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
                let _span = tracing::debug_span!(
                    target: "impasto::io",
                    "image_upload",
                    request_id = request.generation
                )
                .entered();
                let factory = lock(upload).clone();
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
    plugins: Option<crate::plugins::PluginSession>,
    // WindowSession keeps a weak subscription to this poster.
    _window: Arc<Poster>,
    latest: Option<Arc<engine::scene::LayerTree>>,
    image_requirements: Arc<[(Arc<str>, bool)]>,
    input: Input,
    scheduler: FrameScheduler,
    vsync: Arc<dyn crate::Vsync>,
    last_vsync: Option<FrameTime>,
    compositor_mode: Arc<compositor_animation::CompositorMode>,
    compositor_enabled: bool,
    compositor_epoch: u64,
    compositor_generation: Option<u64>,
    animated_effects: bool,
    gpu_views: bool,
    point_scale: f32,
    frame_number: u64,
    interaction: Option<u64>,
    queue_placement: &'static str,
    handoff_ready: bool,
    telemetry: Option<Arc<TelemetryHub>>,
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    _remote: Option<crate::diagnostics::remote::Server>,
    gpu_poll: bool,
    semantics: Option<crate::semantics::Projection>,
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
        let result = self.update_raster_checked(work, move |runtime, result| {
            apply(runtime, result);
            Ok(())
        });
        debug_assert!(result.is_ok(), "infallible raster update");
    }

    fn update_raster_checked<R: Send + 'static>(
        &mut self,
        work: impl FnOnce(&mut RasterHost) -> R + Send + 'static,
        apply: impl FnOnce(&mut Self, R) -> Result<(), BootError> + Send + 'static,
    ) -> Result<(), BootError> {
        match &mut self.graphics {
            RasterPort::Local(graphics) => {
                let result = work(graphics);
                apply(self, result)
            }
            RasterPort::Remote(_) => {
                let inbox = Arc::clone(&self.inbox);
                self.graphics.post(move |graphics| {
                    let result = work(graphics);
                    inbox.complete(Box::new(move |ui| {
                        apply(ui, result)?;
                        Ok(false)
                    }));
                });
                Ok(())
            }
        }
    }
}

impl UiHost {
    fn cancel_vsync(&mut self) {
        if let Some(token) = self.scheduler.cancel() {
            self.vsync.cancel_if_token(token);
        }
    }
}

impl Drop for UiHost {
    fn drop(&mut self) {
        self.compositor_mode.cancel(self.vsync.as_ref());
        self.cancel_vsync();
        self.session.take();
        self.plugins.take();
        self.graphics.retire();
    }
}

#[derive(Default)]
struct FramePipeline {
    in_flight: usize,
    waiting: bool,
    pending: Option<Box<FrameSubmission>>,
}

impl FramePipeline {
    const DEPTH: usize = 2;

    fn has_room(&mut self, depth: usize) -> bool {
        let room = self.in_flight < depth;
        self.waiting |= !room;
        room
    }

    fn keep_latest(&mut self, mut next: FrameSubmission) {
        if let Some(mut previous) = self.pending.take() {
            next.layers_changed |= previous.layers_changed;
            next.placements_changed |= previous.placements_changed;
            if next.census.is_none() {
                next.census = previous.census.take();
            }
            if previous.layers_changed {
                next.scene_delta = None;
            }
            previous.reloads.extend(next.reloads);
            previous.reloads.sort_unstable();
            previous.reloads.dedup();
            next.reloads = previous.reloads;
        }
        self.pending = Some(Box::new(next));
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
    shell: &Arc<Shell>,
    startup: &Startup,
    runners: TaskRunners,
    links: RenderLinks,
) -> Result<UiHost, BootError> {
    startup.checkpoint()?;
    let inbox = &shell.inbox;
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
    let resource_inbox = Arc::downgrade(inbox);
    let surface_gate = inbox.surface_gate();
    let create = move || {
        let mut raster = open(library).map_err(BootError::Engine)?;
        raster.set_resource_wake(move || {
            if let Some(inbox) = resource_inbox.upgrade() {
                inbox.complete(Box::new(|runtime| {
                    runtime.update_raster(
                        |graphics| {
                            let ready = graphics.surfaces.is_open()
                                && graphics.raster.borrow_mut().collect_resources();
                            graphics.needs_present |= ready;
                            ready
                        },
                        |runtime, ready| {
                            if ready && !runtime.input.is_paused() {
                                arm(runtime);
                            }
                        },
                    );
                    Ok(false)
                }));
            }
        });
        if links.enable_gpu_timing {
            raster.enable_gpu_timing();
        }
        Ok::<_, BootError>(RasterHost {
            raster: Rc::new(RefCell::new(raster)),
            telemetry: raster_telemetry,
            gpu_poll_pending: false,
            surfaces: SurfaceHost::with_gate(surface_gate),
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
    let (graphics, raster_ready) = if split {
        let slot = Arc::new(RunnerSlot::new(Arc::clone(&runners.raster)));
        let target = Arc::clone(&slot);
        let (ready, response) = mpsc::sync_channel(1);
        runners.raster.post(Box::new(move || {
            let result = create().map(|raster| target.install(raster));
            let _ = ready.send(result);
        }));
        (RasterPort::Remote(slot), Some(response))
    } else {
        (RasterPort::Local(Box::new(create()?)), None)
    };
    startup.checkpoint()?;
    let open = match boot.launch {
        Launch::Session(open) => open,
        Launch::Migrating(factory) => factory.0,
    };
    let parts = crate::SessionOpen {
        shaper: Box::new(Rc::clone(&shaper)),
        log: console_log(),
    };
    let window_session = services.borrow().window_session();
    let window = Arc::new(Poster {
        inbox: Arc::clone(inbox),
    });
    window_session.connect(&window);
    let bindings = crate::plugins::WindowBindings {
        initial_url: engine::session::shared_read(&boot.launch_url),
        session: window_session,
        restoration: services.borrow().restoration(),
        os_version: boot
            .devtools
            .as_ref()
            .map_or_else(String::new, |_| host_os_version()),
        diagnostics: links.telemetry.clone(),
        source_map: boot.devtools_source_map,
    };
    let (registry, plugins) = crate::plugins::attach(
        boot.modules,
        boot.plugin_services,
        bindings,
        inbox,
        Arc::clone(&runners.io),
        boot.platform_runner
            .as_ref()
            .map(|_| Arc::clone(&runners.platform)),
    )?;
    let registry = Arc::new(registry);
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
        asset_reader: Some(Arc::new({
            let decoder = Arc::clone(&boot.decoder);
            move |relative, limit| decoder.read_packaged_asset(relative, limit)
        })),
        script_execution: if runners.ui.queue_id() == runners.platform.queue_id()
            && boot.platform_runner.is_some()
        {
            engine::session::ScriptExecution::PlatformUi
        } else {
            engine::session::ScriptExecution::DedicatedUi
        },
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
    if let Some(ready) = raster_ready {
        ready.recv().map_err(|_| BootError::Panicked)??;
    }
    let frame_shell = Arc::downgrade(shell);
    let platform = Arc::clone(&runners.platform);
    let inbox_frames = Arc::clone(inbox);
    let frame_clock = Arc::clone(&runners.ui);
    let compositor_mode = Arc::new(compositor_animation::CompositorMode::default());
    let route_mode = Arc::downgrade(&compositor_mode);
    let render_queue = match &graphics {
        RasterPort::Remote(slot) => Some(Arc::downgrade(slot)),
        RasterPort::Local(_) => None,
    };
    boot.vsync.bind(Arc::new(move |token, mut time| {
        time.mono_ms = frame_clock.monotonic_ms();
        if compositor_animation::CompositorMode::is_raster_token(token) {
            if let Some(slot) = render_queue.as_ref().and_then(std::sync::Weak::upgrade)
                && let Some(route_mode) = route_mode.upgrade()
            {
                route_mode.route(&slot, &inbox_frames, token, time);
            }
        } else {
            let inline = frame_shell.upgrade().filter(|shell| {
                platform.runs_on_current_thread()
                    && shell.ui.runner().runs_on_current_thread()
                    && !shell.draining.load(Ordering::Acquire)
            });
            let command = Command::Frame { token, time };
            if let Some(shell) = inline {
                inbox_frames.post_inline_frame(command);
                shell.drain(64);
            } else {
                inbox_frames.post(command);
            }
        }
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
        plugins,
        _window: window,
        latest: None,
        image_requirements: Arc::default(),
        input: Input::new(),
        scheduler: FrameScheduler::new(),
        vsync: boot.vsync,
        last_vsync: None,
        compositor_mode,
        compositor_enabled: split && std::env::var("IMPASTO_COMPOSITOR").as_deref() != Ok("0"),
        compositor_epoch: 0,
        compositor_generation: None,
        animated_effects: false,
        gpu_views: false,
        point_scale: 1.0,
        frame_number: 0,
        interaction: None,
        queue_placement: if !split {
            "shared"
        } else if runners.ui.queue_id() == runners.platform.queue_id() {
            if boot.platform_runner.is_some() {
                "platform-ui"
            } else {
                "dedicated-ui"
            }
        } else {
            "ui-boot"
        },
        handoff_ready: false,
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
    match session.apply_source_patch(source) {
        Ok(false) => {}
        Ok(true) => {
            arm(runtime);
            runtime.services.borrow().source_updated(revision, Ok(()));
            return;
        }
        Err(error) => {
            runtime
                .services
                .borrow()
                .source_updated(revision, Err(&error));
            return;
        }
    }
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

fn on_command(runtime: &mut UiHost, command: Command, acks: &Arc<Acks>) -> Result<bool, BootError> {
    runtime.busy.enter(crate::command::command_tag(&command));
    if let Command::Deadline { ticket } = command {
        let Some(present_retry) = runtime.wake.accept(ticket) else {
            runtime.busy.leave();
            return Ok(false);
        };
        if !present_retry
            && let Some(generation) = runtime.compositor_generation
            && let Some(session) = runtime.session.as_mut()
        {
            use engine::session::ScriptInvalidation;

            let now = runtime.wake.runner.monotonic_ms();
            match session.advance_script_deadline(crate::timing::ms_i64(now)) {
                ScriptInvalidation::Unchanged => {
                    refresh_wake(runtime);
                    runtime.busy.leave();
                    return Ok(false);
                }
                ScriptInvalidation::Content
                    if runtime.compositor_mode.prepare_content(generation) =>
                {
                    let time = frame_cmds::wake_time(now, runtime.last_vsync);
                    let result = on_frame(runtime, time, false, Some(generation), acks);
                    runtime.busy.leave();
                    return result;
                }
                _ => {}
            }
        }
        runtime.busy.leave();
        return on_command(runtime, Command::Wake, acks);
    }
    if matches!(&command, Command::Input(_) | Command::BackGesture(_))
        || matches!(&command, Command::Pointer(event)
            if matches!(event.phase, engine::input::Phase::Down | engine::input::Phase::Wheel | engine::input::Phase::Zoom))
    {
        static NEXT_INTERACTION: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_INTERACTION.fetch_add(1, Ordering::Relaxed);
        runtime.interaction = Some(id);
        tracing::event!(name: "interaction_received", target: "impasto::frame", tracing::Level::DEBUG,
            trace_event = true, interaction = id, surface = runtime.active_surface.map(|surface| surface.0));
    }
    let wake_command = matches!(command, Command::Wake);
    let prefetch_image = match &command {
        Command::ImageReady(ready) => compositor_animation::prefetch_image(runtime, ready.source()),
        _ => false,
    };
    let interrupted = if prefetch_image
        || matches!(
            command,
            Command::Frame { .. }
                | Command::CommitCompositorState { .. }
                | Command::PrefetchCompositorWindow { .. }
        ) {
        false
    } else {
        runtime.compositor_epoch = runtime.compositor_epoch.wrapping_add(1);
        compositor_animation::stop(runtime, None)
    };
    let result = dispatch_command(runtime, command, acks);
    if interrupted
        && !wake_command
        && matches!(result, Ok(false))
        && !runtime.input.is_paused()
        && !runtime.scheduler.token_armed()
    {
        arm(runtime);
    }
    runtime.busy.leave();
    result
}

fn dispatch_command(
    runtime: &mut UiHost,
    command: Command,
    acks: &Arc<Acks>,
) -> Result<bool, BootError> {
    match command {
        #[cfg(any(debug_assertions, feature = "devtools-server"))]
        command @ Command::Devtools { .. } => session_cmds::handle(runtime, command),
        Command::Shutdown => Ok(true),
        Command::Deadline { .. } => Ok(false),
        Command::UpdateSource { revision, source } => {
            update_source(runtime, &revision, &source);
            Ok(false)
        }
        command @ (Command::SurfaceDestroyed { .. }
        | Command::SurfaceReady(_)
        | Command::SurfaceChanged { .. }) => surface_cmds::handle(runtime, command, acks),
        command @ (Command::Pointer(_)
        | Command::PointerMoves { .. }
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
        Command::PrefetchCompositorWindow { generation, time } => {
            if runtime.compositor_generation == Some(generation)
                && runtime.compositor_mode.running()
                && !runtime.input.is_paused()
            {
                on_frame(runtime, time, false, Some(generation), acks)
            } else {
                Ok(false)
            }
        }
        Command::CommitCompositorState {
            generation,
            snapshot,
            time,
        } => {
            if runtime.compositor_generation != Some(generation) {
                return Ok(false);
            }
            compositor_animation::stop(runtime, snapshot);
            if runtime.input.is_paused() {
                return Ok(false);
            }
            if let Some(time) = time
                && let Some(token) = runtime.scheduler.request()
            {
                return frame_cmds::handle(runtime, Command::Frame { token, time }, acks);
            }
            arm(runtime);
            Ok(false)
        }
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

fn next_frame_number() -> u64 {
    static NEXT_FRAME: AtomicU64 = AtomicU64::new(1);
    NEXT_FRAME.fetch_add(1, Ordering::Relaxed)
}

fn end_frame(runtime: &mut UiHost, again: bool, stop: bool) -> bool {
    if stop {
        runtime.cancel_vsync();
        runtime.wake.set(None);
        return true;
    }
    refresh_wake(runtime);
    let armed = runtime.scheduler.armed();
    if let Some(token) = runtime.scheduler.finish(again) {
        runtime.vsync.request_frame(token);
    } else if let Some(token) = armed.filter(|_| !runtime.scheduler.token_armed()) {
        runtime.vsync.cancel_if_token(token);
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
    let retry = runtime
        .present_retry_at
        .filter(|_| !runtime.input.is_paused());
    let deadline = match (session_deadline, retry) {
        (Some(session), Some(retry)) if session < retry => Some((session, false)),
        (_, Some(retry)) => Some((retry, true)),
        (session, None) => session.map(|at| (at, false)),
    };
    runtime
        .wake
        .set(deadline.filter(|_| !runtime.frames.waiting));
}

struct WakeTimer {
    runner: Arc<dyn TaskRunner>,
    inbox: Arc<Inbox>,
    pending: RefCell<Option<(i64, bool, u64, TaskHandle)>>,
    next_ticket: std::cell::Cell<u64>,
}

impl WakeTimer {
    fn new(runner: Arc<dyn TaskRunner>, inbox: Arc<Inbox>) -> Self {
        Self {
            runner,
            inbox,
            pending: RefCell::default(),
            next_ticket: std::cell::Cell::default(),
        }
    }

    fn set(&self, next: Option<(i64, bool)>) {
        let mut pending = self.pending.borrow_mut();
        if let Some((deadline, retry, ticket, handle)) = pending.take() {
            if next == Some((deadline, retry)) {
                *pending = Some((deadline, retry, ticket, handle));
                return;
            }
            handle.cancel();
        }
        if let Some((deadline, retry)) = next {
            let ticket = self.next_ticket.get().wrapping_add(1);
            self.next_ticket.set(ticket);
            let now = crate::timing::ms_i64(self.runner.monotonic_ms());
            let millis = u64::try_from(deadline.saturating_sub(now)).unwrap_or(0);
            let inbox = Arc::downgrade(&self.inbox);
            let handle = self.runner.post_delayed(
                Duration::from_millis(millis),
                Box::new(move || {
                    if let Some(inbox) = inbox.upgrade() {
                        inbox.post(Command::Deadline { ticket });
                    }
                }),
            );
            *pending = Some((deadline, retry, ticket, handle));
        }
    }

    fn accept(&self, ticket: u64) -> Option<bool> {
        let mut pending = self.pending.borrow_mut();
        if pending.as_ref()?.2 != ticket {
            return None;
        }
        pending.take().map(|(_, retry, _, _)| retry)
    }
}

impl Drop for WakeTimer {
    fn drop(&mut self) {
        self.set(None);
    }
}

fn arm(runtime: &mut UiHost) {
    if runtime.compositor_generation.is_none()
        && let Some(token) = runtime.scheduler.request()
    {
        runtime.vsync.request_frame(token);
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
    if let RasterPort::Remote(slot) = &runtime.graphics {
        raster_host::schedule_gpu_poll(Arc::clone(slot), hub);
        return;
    }
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
    runtime.wake.set(None);
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
mod tests;
