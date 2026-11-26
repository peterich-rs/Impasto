use engine::session::FrameTime;

use super::{Acks, UiHost, arm, on_frame};
use crate::command::{Command, FrameToken};
use crate::error::BootError;

pub(super) fn handle(
    runtime: &mut UiHost,
    command: Command,
    acks: &Acks,
) -> Result<bool, BootError> {
    match command {
        Command::Wake => wake(runtime, acks),
        Command::Frame { token, time } => frame(runtime, token, time, acks),
        _ => Ok(false),
    }
}

fn wake(runtime: &mut UiHost, acks: &Acks) -> Result<bool, BootError> {
    if runtime.input.is_paused() {
        return Ok(false);
    }
    if runtime.scheduler.busy() {
        super::skipped_frame("wake_coalesced");
        arm(runtime);
        return Ok(false);
    }
    // A posted vsync already owns the next frame. Stealing it without `accept`
    // used to cancel that callback and leave the scheduler holding a token the
    // platform can never echo, so later `arm` calls freeze until resume.
    if runtime.scheduler.token_armed() {
        super::skipped_frame("wake_coalesced");
        return Ok(false);
    }
    let Some(token) = runtime.scheduler.request() else {
        return Ok(false);
    };
    if !runtime.scheduler.accept(token) {
        return Ok(false);
    }
    let now = runtime.wake.runner.monotonic_ms();
    on_frame(
        runtime,
        wake_time(now, runtime.last_vsync),
        false,
        None,
        acks,
    )
}

fn wake_time(now: f64, last_vsync: Option<FrameTime>) -> FrameTime {
    // JavaScript deadlines use the runner's clock. Motion uses platform vsync
    // timestamps, whose origin can be the device boot rather than this process.
    let Some(last) = last_vsync else {
        return FrameTime {
            interval_ms: 16.0,
            ..FrameTime::at(now)
        };
    };
    let vsync_ms = last.vsync_ms + (now - last.mono_ms).max(0.0);
    FrameTime {
        vsync_ms,
        target_ms: vsync_ms.max(last.target_ms),
        interval_ms: last.interval_ms,
        mono_ms: now,
    }
}

fn frame(
    runtime: &mut UiHost,
    token: FrameToken,
    time: FrameTime,
    acks: &Acks,
) -> Result<bool, BootError> {
    if !runtime.scheduler.accept(token) {
        super::skipped_frame("stale_vsync");
        return Ok(false);
    }
    runtime.last_vsync = Some(time);
    on_frame(runtime, time, true, None, acks)
}

#[cfg(test)]
mod tests {
    use super::{FrameTime, wake_time};

    #[test]
    fn timer_keeps_display_clock_for_motions_and_process_clock_for_js() {
        let last_vsync = FrameTime {
            vsync_ms: 39_000_000.0,
            target_ms: 39_000_008.0,
            interval_ms: 8.333,
            mono_ms: 220.0,
        };
        let soon = wake_time(221.0, Some(last_vsync));
        assert_eq!(soon.mono_ms, 221.0);
        assert_eq!(soon.target_ms, last_vsync.target_ms);
        let later = wake_time(400.0, Some(last_vsync));
        assert_eq!(later.target_ms, last_vsync.vsync_ms + 180.0);
        assert_eq!(later.interval_ms, last_vsync.interval_ms);
    }
}
