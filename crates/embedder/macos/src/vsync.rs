//! Window-scoped display callbacks deliver one armed frame token.

#![allow(deprecated, reason = "CVDisplayLink supports systems before macOS 14")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, available, define_class, sel};
use objc2_app_kit::NSWindow;
use objc2_foundation::{NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};

use embedder::{FrameRateHint, FrameTime, FrameToken, Vsync};
use objc2_core_foundation::CFRetained;
use objc2_core_video::{
    CVDisplayLink, CVDisplayLinkOutputCallback, CVGetHostClockFrequency, CVTimeStamp,
    kCVReturnSuccess,
};

type FrameSink = Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>;

pub enum DisplayLink {
    Window(WindowClock),
    Legacy(LegacyLink),
}

impl DisplayLink {
    pub fn new() -> anyhow::Result<Self> {
        if available!(macos = 14.0) {
            Ok(Self::Window(WindowClock::new()))
        } else {
            LegacyLink::new().map(Self::Legacy)
        }
    }

    pub fn attach(&self, window: &NSWindow, mtm: MainThreadMarker) {
        if let Self::Window(clock) = self {
            clock.attach(window, mtm);
        }
    }

    fn backend(&self) -> &dyn Vsync {
        match self {
            Self::Window(clock) => clock,
            Self::Legacy(clock) => clock,
        }
    }
}

impl Vsync for DisplayLink {
    fn bind(&self, sink: FrameSink) {
        self.backend().bind(sink);
    }

    fn request_frame(&self, token: FrameToken) {
        self.backend().request_frame(token);
    }

    fn request_frame_with_rate(&self, token: FrameToken, hint: FrameRateHint) {
        self.backend().request_frame_with_rate(token, hint);
    }

    fn cancel_if_token(&self, token: FrameToken) {
        self.backend().cancel_if_token(token);
    }
}

struct WindowState {
    id: u64,
    post: Mutex<Option<FrameSink>>,
    pending: Mutex<Option<(FrameToken, FrameRateHint)>>,
}

pub struct WindowClock(Arc<WindowState>);

thread_local! {
    static WINDOW_LINKS: RefCell<HashMap<u64, Retained<CADisplayLink>>> = RefCell::default();
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Arc<WindowState>]
    struct WindowTarget;

    // SAFETY: NSObject methods are inherited. The callback stays on AppKit's main run loop.
    unsafe impl NSObjectProtocol for WindowTarget {}

    impl WindowTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            link.setPaused(true);
            let state = self.ivars();
            let request = state.pending.lock().unwrap_or_else(|error| error.into_inner()).take();
            let Some((token, _)) = request else { return };
            let sink = state.post.lock().unwrap_or_else(|error| error.into_inner()).clone();
            if let Some(sink) = sink {
                let vsync_ms = link.timestamp() * 1000.0;
                let target_ms = link.targetTimestamp() * 1000.0;
                sink(token, FrameTime { vsync_ms, target_ms, interval_ms: target_ms - vsync_ms, mono_ms: 0.0 });
            }
        }
    }
);

impl WindowClock {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(WindowState {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            post: Mutex::new(None),
            pending: Mutex::new(None),
        }))
    }

    fn attach(&self, window: &NSWindow, mtm: MainThreadMarker) {
        let target = WindowTarget::alloc(mtm).set_ivars(Arc::clone(&self.0));
        // SAFETY: WindowTarget is a valid NSObject subclass and its tick selector accepts CADisplayLink.
        let target: Retained<WindowTarget> = unsafe { objc2::msg_send![super(target), init] };
        // SAFETY: the target is retained by the display link for this selector.
        let link = unsafe { window.displayLinkWithTarget_selector(&target, sel!(tick:)) };
        link.setPaused(true);
        // SAFETY: This code runs on the main thread and the AppKit link is scoped to its window.
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSRunLoopCommonModes) };
        WINDOW_LINKS.with_borrow_mut(|links| links.insert(self.0.id, link));
        self.0.update_link();
    }

    fn dispatch_update(&self) {
        let state = Arc::clone(&self.0);
        crate::app::on_main(Box::new(move || state.update_link()));
    }
}

impl WindowState {
    fn update_link(&self) {
        let request = *self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        WINDOW_LINKS.with_borrow(|links| {
            let Some(link) = links.get(&self.id) else {
                return;
            };
            if let Some((_, hint)) = request {
                let rate = match hint {
                    FrameRateHint::Display => 120.0,
                    FrameRateHint::Hz60 => 60.0,
                    FrameRateHint::Hz30 => 30.0,
                };
                let range = CAFrameRateRange {
                    minimum: 30.0,
                    maximum: rate,
                    preferred: rate,
                };
                if link.preferredFrameRateRange() != range {
                    link.setPreferredFrameRateRange(range);
                }
            }
            link.setPaused(request.is_none());
        });
    }
}

impl Vsync for WindowClock {
    fn bind(&self, sink: FrameSink) {
        *self
            .0
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(sink);
    }

    fn request_frame(&self, token: FrameToken) {
        self.request_frame_with_rate(token, FrameRateHint::Display);
    }

    fn request_frame_with_rate(&self, token: FrameToken, hint: FrameRateHint) {
        *self
            .0
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some((token, hint));
        self.dispatch_update();
    }

    fn cancel_if_token(&self, token: FrameToken) {
        {
            let mut pending = self
                .0
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if pending
                .as_ref()
                .is_none_or(|(current, _)| *current != token)
            {
                return;
            }
            *pending = None;
        }
        self.dispatch_update();
    }
}

impl Drop for WindowClock {
    fn drop(&mut self) {
        let id = self.0.id;
        crate::app::on_main(Box::new(move || {
            if let Some(link) = WINDOW_LINKS.with_borrow_mut(|links| links.remove(&id)) {
                link.invalidate();
            }
        }));
    }
}

struct Callback {
    post: Mutex<Option<FrameSink>>,
    token: Mutex<Option<FrameToken>>,
}

pub struct LegacyLink {
    link: CFRetained<CVDisplayLink>,
    callback: *mut Callback,
    control: Mutex<()>,
}

#[allow(
    unsafe_code,
    reason = "Send follows the CVDisplayLink contract on the next line"
)]
// SAFETY: The host hands start/request ownership between UI and raster serial queues.
// CoreVideo's stop waits until the callback has returned before the context is released.
unsafe impl Send for LegacyLink {}
#[allow(
    unsafe_code,
    reason = "Sync follows the CVDisplayLink contract on the next line"
)]
// SAFETY: The host serializes start and stop, and all callback state is protected by mutexes.
unsafe impl Sync for LegacyLink {}

impl LegacyLink {
    pub fn new() -> anyhow::Result<Self> {
        let mut raw = ptr::null_mut();
        let status = {
            // SAFETY: `raw` is a stack slot CoreVideo writes; success returns one owned reference.
            unsafe { CVDisplayLink::create_with_active_cg_displays(NonNull::from(&mut raw)) }
        };
        if status != kCVReturnSuccess {
            anyhow::bail!("CVDisplayLink was not created: {status}");
        }
        let Some(raw) = NonNull::new(raw) else {
            anyhow::bail!("CVDisplayLink was not created");
        };
        let callback = Box::into_raw(Box::new(Callback {
            post: Mutex::new(None),
            token: Mutex::new(None),
        }));
        // SAFETY: `raw` is the link just created, and `callback` lives until Drop stops the link.
        let set = unsafe {
            CVDisplayLink::set_output_callback(
                raw.as_ref(),
                Some(on_frame),
                callback.cast::<c_void>(),
            )
        };
        if set != kCVReturnSuccess {
            // SAFETY: the callback was not installed, so CoreVideo does not hold this box.
            drop(unsafe { Box::from_raw(callback) });
            anyhow::bail!("CVDisplayLink callback was not set: {set}");
        }
        Ok(Self {
            // SAFETY: `raw` is the +1 reference from a successful create and is not aliased.
            link: unsafe { CFRetained::from_raw(raw) },
            callback,
            control: Mutex::default(),
        })
    }
}

impl Vsync for LegacyLink {
    fn bind(&self, post_frame: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>) {
        // SAFETY: `callback` was allocated in `new` and is freed in `Drop` only after `stop`.
        let callback = unsafe { &*self.callback };
        *callback
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(post_frame);
    }

    fn request_frame(&self, token: FrameToken) {
        let _control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // SAFETY: `callback` was allocated in `new` and is freed in `Drop` only after `stop`.
        let callback = unsafe { &*self.callback };
        *callback
            .token
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(token);
        if self.link.is_running() {
            return;
        }
        let _ = self.link.start();
    }

    fn cancel_if_token(&self, token: FrameToken) {
        let _control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        {
            // SAFETY: the callback box is owned by this link until Drop.
            let callback = unsafe { &*self.callback };
            let mut pending = callback
                .token
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *pending != Some(token) {
                return;
            }
            *pending = None;
        }
        if self.link.is_running() {
            let _ = self.link.stop();
        }
    }
}

impl Drop for LegacyLink {
    fn drop(&mut self) {
        if self.link.is_running() {
            let _ = self.link.stop();
        }
        // SAFETY: `stop` has returned, so this is the only remaining owner of the callback box.
        drop(unsafe { Box::from_raw(self.callback) });
    }
}

// SAFETY: CoreVideo calls this with the pointer passed to `set_output_callback`.
unsafe extern "C-unwind" fn on_frame(
    _link: NonNull<CVDisplayLink>,
    _now: NonNull<CVTimeStamp>,
    output: NonNull<CVTimeStamp>,
    _flags_in: u64,
    _flags_out: NonNull<u64>,
    context: *mut c_void,
) -> i32 {
    // SAFETY: `context` is the Callback box installed as the display-link output callback.
    let callback = unsafe { &*context.cast::<Callback>() };
    let token = callback
        .token
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    let Some(token) = token else {
        return kCVReturnSuccess;
    };
    let frequency = CVGetHostClockFrequency();
    // SAFETY: CoreVideo passes live timestamps for this callback invocation.
    let now = unsafe { _now.as_ref() };
    // SAFETY: `output` is the frame timestamp CoreVideo passes beside `_now`.
    let shown = unsafe { output.as_ref() };
    let time = FrameTime {
        vsync_ms: host_ms(now, frequency),
        target_ms: host_ms(shown, frequency),
        interval_ms: refresh_ms(shown),
        mono_ms: 0.0,
    };
    if let Some(post) = callback
        .post
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        post(token, time);
    }
    kCVReturnSuccess
}

fn host_ms(stamp: &CVTimeStamp, frequency: f64) -> f64 {
    if frequency > 0.0 {
        stamp.hostTime as f64 / frequency * 1000.0
    } else {
        0.0
    }
}

fn refresh_ms(stamp: &CVTimeStamp) -> f64 {
    if stamp.videoTimeScale <= 0 {
        return 0.0;
    }
    stamp.videoRefreshPeriod as f64 / f64::from(stamp.videoTimeScale) * 1000.0
}

// The generated binding's callback alias includes `CVOptionFlags`.
const _: CVDisplayLinkOutputCallback = Some(on_frame);
