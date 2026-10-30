//! One Choreographer callback per armed frame. Java posts it on the UI thread.
//! The callback only enqueues `Frame`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hostshell::{FrameTime, FrameToken, Vsync};

type FrameSink = Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>;

struct Inner {
    post: Mutex<Option<FrameSink>>,
    armed: AtomicBool,
    token: AtomicU64,
    generation: AtomicU64,
    posted: AtomicU64,
}

pub struct FrameClock {
    inner: Arc<Inner>,
    schedule: Arc<dyn Fn() + Send + Sync>,
}

impl FrameClock {
    pub fn new(schedule: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                post: Mutex::new(None),
                armed: AtomicBool::new(false),
                token: AtomicU64::new(0),
                generation: AtomicU64::new(1),
                posted: AtomicU64::new(0),
            }),
            schedule,
        })
    }

    /// Called on the UI thread from `VsyncWaiter`.
    pub fn on_vsync(&self, frame_time_nanos: i64, target_nanos: i64, interval_nanos: i64) {
        let posted = self.inner.posted.load(Ordering::SeqCst);
        let generation = self.inner.generation.load(Ordering::SeqCst);
        if posted != generation {
            if self.inner.armed.load(Ordering::SeqCst) {
                self.arm();
            }
            return;
        }
        let token = self.inner.token.swap(0, Ordering::SeqCst);
        if token == 0 {
            return;
        }
        let Some(post) = self
            .inner
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        else {
            return;
        };
        let vsync_ms = frame_time_nanos as f64 / 1_000_000.0;
        let target_ms = target_nanos as f64 / 1_000_000.0;
        let interval_ms = if interval_nanos > 0 {
            interval_nanos as f64 / 1_000_000.0
        } else if target_ms > vsync_ms {
            target_ms - vsync_ms
        } else {
            1000.0 / 60.0
        };
        post(
            FrameToken(token),
            FrameTime {
                vsync_ms,
                target_ms: if target_ms > vsync_ms {
                    target_ms
                } else {
                    vsync_ms + interval_ms
                },
                interval_ms,
                mono_ms: 0.0,
            },
        );
    }

    fn arm(&self) {
        let generation = self.inner.generation.load(Ordering::SeqCst);
        self.inner.posted.store(generation, Ordering::SeqCst);
        (self.schedule)();
    }
}

impl Vsync for FrameClock {
    fn bind(&self, post_frame: FrameSink) {
        *self
            .inner
            .post
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(post_frame);
    }

    fn request_frame(&self, token: FrameToken) {
        self.inner.token.store(token.0, Ordering::SeqCst);
        self.inner.armed.store(true, Ordering::SeqCst);
        self.arm();
    }

    fn cancel(&self) {
        self.inner.token.store(0, Ordering::SeqCst);
        self.inner.armed.store(false, Ordering::SeqCst);
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
    }
}
