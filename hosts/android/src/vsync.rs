//! One Choreographer callback per `request_frame`. The callback only posts `Frame`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use android_activity::AndroidAppWaker;
use hostshell::Vsync;

#[repr(C)]
struct Choreographer {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn AChoreographer_getInstance() -> *mut Choreographer;
    fn AChoreographer_postFrameCallback64(
        choreographer: *mut Choreographer,
        callback: Option<unsafe extern "C" fn(frame_time_nanos: i64, data: *mut c_void)>,
        data: *mut c_void,
    );
}

struct Inner {
    post: Mutex<Option<Arc<dyn Fn(f64) + Send + Sync>>>,
    armed: AtomicBool,
    generation: AtomicU64,
    posted: AtomicU64,
    choreographer: AtomicPtr<Choreographer>,
}

pub struct FrameClock {
    inner: Arc<Inner>,
    waker: AndroidAppWaker,
}

impl FrameClock {
    pub fn new(waker: AndroidAppWaker) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                post: Mutex::new(None),
                armed: AtomicBool::new(false),
                generation: AtomicU64::new(1),
                posted: AtomicU64::new(0),
                choreographer: AtomicPtr::new(std::ptr::null_mut()),
            }),
            waker,
        })
    }

    /// Called on the `android_main` looper thread after it wakes.
    pub fn arm_if_needed(&self) {
        if !self.inner.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        let choreographer = self.choreographer();
        if choreographer.is_null() {
            return;
        }
        let generation = self.inner.generation.load(Ordering::SeqCst);
        self.inner.posted.store(generation, Ordering::SeqCst);
        // SAFETY: `choreographer` came from `AChoreographer_getInstance` on this looper
        // thread. `inner` stays alive until this process exits. The callback only reads it.
        unsafe {
            AChoreographer_postFrameCallback64(
                choreographer,
                Some(on_frame),
                Arc::as_ptr(&self.inner).cast_mut().cast(),
            );
        }
    }

    fn choreographer(&self) -> *mut Choreographer {
        let existing = self.inner.choreographer.load(Ordering::SeqCst);
        if !existing.is_null() {
            return existing;
        }
        // SAFETY: this method runs on the looper thread. The instance pointer stays
        // valid for the process.
        let created = unsafe { AChoreographer_getInstance() };
        if !created.is_null() {
            self.inner.choreographer.store(created, Ordering::SeqCst);
        }
        created
    }
}

impl Vsync for FrameClock {
    fn bind(&self, post_frame: Arc<dyn Fn(f64) + Send + Sync>) {
        *self
            .inner
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(post_frame);
    }

    fn request_frame(&self) {
        self.inner.armed.store(true, Ordering::SeqCst);
        self.waker.wake();
    }

    fn cancel(&self) {
        self.inner.armed.store(false, Ordering::SeqCst);
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.waker.wake();
    }
}

unsafe extern "C" fn on_frame(frame_time_nanos: i64, data: *mut c_void) {
    let inner = unsafe { &*data.cast::<Inner>() };
    let posted = inner.posted.load(Ordering::SeqCst);
    let generation = inner.generation.load(Ordering::SeqCst);
    if posted != generation {
        return;
    }
    let Some(post) = inner
        .post
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    else {
        return;
    };
    post(frame_time_nanos as f64 / 1_000_000.0);
}
