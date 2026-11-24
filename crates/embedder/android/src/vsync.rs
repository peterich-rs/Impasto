//! Choreographer returns the token that scheduled each callback.

use std::sync::{Arc, Mutex};

use embedder::{FrameRateHint, FrameTime, FrameToken, Vsync};

type FrameSink = Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>;

#[derive(Default)]
struct ClockState {
    post: Option<FrameSink>,
    pending: Option<FrameToken>,
}

pub struct FrameClock {
    state: Mutex<ClockState>,
    schedule: Arc<dyn Fn(FrameToken, FrameRateHint) + Send + Sync>,
}

impl FrameClock {
    pub fn new(schedule: Arc<dyn Fn(FrameToken, FrameRateHint) + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ClockState::default()),
            schedule,
        })
    }

    pub fn on_vsync(
        &self,
        token: FrameToken,
        frame_time_nanos: i64,
        target_nanos: i64,
        interval_nanos: i64,
    ) {
        let post = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.pending != Some(token) {
                return;
            }
            state.pending = None;
            state.post.clone()
        };
        let Some(post) = post else { return };
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
            token,
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
}

impl Vsync for FrameClock {
    fn bind(&self, post_frame: FrameSink) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .post = Some(post_frame);
    }

    fn request_frame(&self, token: FrameToken) {
        self.request_frame_with_rate(token, FrameRateHint::Display);
    }

    fn request_frame_with_rate(&self, token: FrameToken, hint: FrameRateHint) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending = Some(token);
        (self.schedule)(token, hint);
    }

    fn cancel(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use embedder::{FrameToken, Vsync};

    use super::FrameClock;

    #[test]
    fn cancelled_or_duplicate_callbacks_cannot_consume_a_new_request() {
        let scheduled = Arc::new(Mutex::new(Vec::new()));
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let schedule_log = Arc::clone(&scheduled);
        let frame_log = Arc::clone(&delivered);
        let clock = FrameClock::new(Arc::new(move |token, _| {
            schedule_log
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(token);
        }));
        clock.bind(Arc::new(move |token, _| {
            frame_log
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(token);
        }));

        clock.request_frame(FrameToken(1));
        clock.cancel();
        clock.request_frame(FrameToken(2));
        clock.on_vsync(FrameToken(1), 1_000_000, 2_000_000, 1_000_000);
        clock.on_vsync(FrameToken(2), 2_000_000, 3_000_000, 1_000_000);
        clock.on_vsync(FrameToken(2), 3_000_000, 4_000_000, 1_000_000);

        assert_eq!(
            *scheduled.lock().unwrap_or_else(|error| error.into_inner()),
            [FrameToken(1), FrameToken(2)],
        );
        assert_eq!(
            *delivered.lock().unwrap_or_else(|error| error.into_inner()),
            [FrameToken(2)],
        );
    }
}
