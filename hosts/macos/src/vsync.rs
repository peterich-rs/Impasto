#![allow(deprecated)]
//! CVDisplayLink posts one `Frame` command onto the render thread.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex};

use hostshell::Vsync;
use objc2_core_foundation::CFRetained;
use objc2_core_video::{
    CVDisplayLink, CVDisplayLinkOutputCallback, CVGetHostClockFrequency, CVTimeStamp,
    kCVReturnSuccess,
};

struct Callback {
    post: Mutex<Option<Arc<dyn Fn(f64) + Send + Sync>>>,
}

pub struct DisplayLink {
    link: CFRetained<CVDisplayLink>,
    callback: *mut Callback,
}

// SAFETY: `CVDisplayLinkStart` and `Stop` are safe to call from the render
// thread. `Stop` returns only after an in-flight callback has finished, so
// dropping the callback pointer does not race the callback.
#[allow(unsafe_code)]
unsafe impl Send for DisplayLink {}
// SAFETY: start and stop do not run at the same time as each other from this
// process; the shell calls them on the render thread only.
#[allow(unsafe_code)]
unsafe impl Sync for DisplayLink {}

impl DisplayLink {
    pub fn new() -> anyhow::Result<Self> {
        let mut raw = ptr::null_mut();
        // SAFETY: `raw` is a stack slot CoreVideo writes the new link into.
        // A successful create returns an owned +1 reference.
        let status =
            unsafe { CVDisplayLink::create_with_active_cg_displays(NonNull::from(&mut raw)) };
        if status != kCVReturnSuccess {
            anyhow::bail!("CVDisplayLink was not created: {status}");
        }
        let Some(raw) = NonNull::new(raw) else {
            anyhow::bail!("CVDisplayLink was not created");
        };
        let callback = Box::into_raw(Box::new(Callback {
            post: Mutex::new(None),
        }));
        // SAFETY: `raw` is the link just created. The callback pointer stays
        // alive until `Drop` stops the link and frees it.
        let set = unsafe {
            CVDisplayLink::set_output_callback(
                raw.as_ref(),
                Some(on_frame),
                callback.cast::<c_void>(),
            )
        };
        if set != kCVReturnSuccess {
            drop(unsafe { Box::from_raw(callback) });
            anyhow::bail!("CVDisplayLink callback was not set: {set}");
        }
        Ok(Self {
            link: unsafe { CFRetained::from_raw(raw) },
            callback,
        })
    }
}

impl Vsync for DisplayLink {
    fn bind(&self, post_frame: Arc<dyn Fn(f64) + Send + Sync>) {
        let callback = unsafe { &*self.callback };
        *callback
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(post_frame);
    }

    fn request_frame(&self) {
        if self.link.is_running() {
            return;
        }
        let _ = self.link.start();
    }

    fn cancel(&self) {
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
        drop(unsafe { Box::from_raw(self.callback) });
    }
}

unsafe extern "C-unwind" fn on_frame(
    _link: NonNull<CVDisplayLink>,
    _now: NonNull<CVTimeStamp>,
    output: NonNull<CVTimeStamp>,
    _flags_in: u64,
    _flags_out: NonNull<u64>,
    context: *mut c_void,
) -> i32 {
    let callback = unsafe { &*context.cast::<Callback>() };
    let frequency = CVGetHostClockFrequency();
    let time_ms = if frequency > 0.0 {
        unsafe { output.as_ref() }.hostTime as f64 / frequency * 1000.0
    } else {
        0.0
    };
    if let Some(post) = callback
        .post
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        post(time_ms);
    }
    kCVReturnSuccess
}

// The generated binding's callback alias includes `CVOptionFlags`.
const _: CVDisplayLinkOutputCallback = Some(on_frame);
