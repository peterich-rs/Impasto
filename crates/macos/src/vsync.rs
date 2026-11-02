//! CVDisplayLink posts one `Frame` command onto the render thread.

#![allow(
    deprecated,
    reason = "CVDisplayLink is the vsync source this host already registers"
)]

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex};

use hostshell::{FrameTime, FrameToken, Vsync};
use objc2_core_foundation::CFRetained;
use objc2_core_video::{
    CVDisplayLink, CVDisplayLinkOutputCallback, CVGetHostClockFrequency, CVTimeStamp,
    kCVReturnSuccess,
};

type FrameSink = Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>;

struct Callback {
    post: Mutex<Option<FrameSink>>,
    token: Mutex<Option<FrameToken>>,
}

pub struct DisplayLink {
    link: CFRetained<CVDisplayLink>,
    callback: *mut Callback,
}

#[allow(
    unsafe_code,
    reason = "Send follows the CVDisplayLink contract on the next line"
)]
// SAFETY: Start and Stop run on the render thread, and Stop waits until the callback returns.
unsafe impl Send for DisplayLink {}
#[allow(
    unsafe_code,
    reason = "Sync follows the CVDisplayLink contract on the next line"
)]
// SAFETY: The shell starts and stops the link on the render thread; the callback only locks the mutex.
unsafe impl Sync for DisplayLink {}

impl DisplayLink {
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
        })
    }
}

impl Vsync for DisplayLink {
    fn bind(&self, post_frame: Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>) {
        // SAFETY: `callback` was allocated in `new` and is freed in `Drop` only after `stop`.
        let callback = unsafe { &*self.callback };
        *callback
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(post_frame);
    }

    fn request_frame(&self, token: FrameToken) {
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

    fn cancel(&self) {
        // SAFETY: `callback` was allocated in `new` and is freed in `Drop` only after `stop`.
        let callback = unsafe { &*self.callback };
        *callback
            .token
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        if self.link.is_running() {
            let _ = self.link.stop();
        }
    }
}

impl Drop for DisplayLink {
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
