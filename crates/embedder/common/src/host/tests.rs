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
use crate::{NativePtr, Presented, Surface, Vsync};

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

    fn cancel_if_token(&self, _token: FrameToken) {
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
    acks: Arc<Acks>,
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
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            overlay_owner: None,
            shaper,
            graphics: RasterPort::Local(Box::new(RasterHost {
                raster,
                telemetry: None,
                gpu_poll_pending: false,
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
            plugins: None,
            registry: Arc::new(engine::session::ModuleRegistry::new(Vec::new())),
            _window: Arc::new(super::Poster {
                inbox: Arc::new(Inbox::new()),
            }),
            latest: None,
            image_requirements: Arc::default(),
            input: crate::input::Input::new(),
            scheduler: FrameScheduler::new(),
            vsync: Arc::clone(&vsync) as Arc<dyn Vsync>,
            last_vsync: None,
            compositor_mode: Arc::default(),
            compositor_enabled: false,
            compositor_epoch: 0,
            compositor_generation: None,
            animated_effects: false,
            gpu_views: false,
            point_scale: 1.0,
            frame_number: 0,
            interaction: None,
            queue_placement: "shared",
            handoff_ready: false,
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
            acks: Arc::new(Acks::new()),
            vsync,
            session: log,
        }
    }

    fn command(&mut self, command: Command) -> bool {
        on_command(&mut self.runtime, command, &self.acks).unwrap_or_else(|error| panic!("{error}"))
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
fn a_full_frame_pipeline_runs_ui_and_keeps_the_latest_submission() {
    let mut harness = TestHarness::new();
    harness.open_dummy();
    assert!(harness.runtime.frames.has_room(FramePipeline::DEPTH));
    harness.runtime.frames.submit();
    assert!(harness.runtime.frames.has_room(FramePipeline::DEPTH));
    harness.runtime.frames.submit();
    let token = harness
        .vsync
        .last_token()
        .expect("surface requests a frame");
    assert!(!harness.command(Command::Frame { token, time: now() }));
    assert_eq!(harness.frame_count(), 1);
    assert_eq!(harness.runtime.frames.in_flight, FramePipeline::DEPTH);
    assert_eq!(
        harness
            .runtime
            .frames
            .pending
            .as_ref()
            .map(|frame| frame.number),
        Some(harness.runtime.frame_number)
    );
    assert!(harness.runtime.frames.complete());
    assert!(harness.runtime.frames.has_room(FramePipeline::DEPTH));
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
    harness.runtime.cancel_vsync();
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
    assert!(!on_frame(&mut harness.runtime, now(), false, None, &harness.acks).unwrap());
    assert_eq!(presents.load(Ordering::Relaxed), 0);
    harness.runtime.graphics.local().gpu = Gpu::Dead;
    harness.runtime.input.resume();
    assert!(!on_frame(&mut harness.runtime, now(), false, None, &harness.acks).unwrap());
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
    assert!(!on_frame(&mut harness.runtime, now(), false, None, &harness.acks).unwrap());
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
    let mut registrar = impasto::PluginRegistrar::default();
    impasto_clipboard::register(&mut registrar);
    let modules = registrar.build(&impasto::PluginContext::default());
    let Ok(modules) = modules else {
        panic!("build clipboard modules");
    };
    let mut harness = TestHarness::new();
    harness.runtime.registry = Arc::new(engine::session::ModuleRegistry::new(modules));
    harness.open_dummy();
    harness.session.borrow_mut().clipboard = Some("copied".into());
    let token = match harness.vsync.last_token() {
        Some(token) => token,
        None => panic!("SurfaceReady arms one vsync token"),
    };
    assert!(!harness.command(Command::Frame { token, time: now() }));
    match harness.runtime.registry.call("Clipboard", "getString", &[]) {
        Ok(engine::session::PluginValue::Str(text)) => assert_eq!(text, "copied"),
        other => panic!("unexpected clipboard contents {other:?}"),
    }
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
    harness.runtime.graphics.local().open_surface = Box::new(|_, _| Err(raster::Status::NotReady));
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
