use session::FrameTime;

use super::{Acks, Runtime, arm, on_frame};
use crate::command::{Command, FrameToken};
use crate::error::BootError;

pub(super) fn handle(
    runtime: &mut Runtime,
    command: Command,
    acks: &Acks,
) -> Result<bool, BootError> {
    match command {
        Command::Wake => wake(runtime, acks),
        Command::Frame { token, time } => frame(runtime, token, time, acks),
        _ => Ok(false),
    }
}

fn wake(runtime: &mut Runtime, acks: &Acks) -> Result<bool, BootError> {
    if runtime.input.is_paused() {
        return Ok(false);
    }
    if runtime.scheduler.busy() {
        arm(runtime);
        return Ok(false);
    }
    // A posted vsync already owns the next frame. Stealing it without `accept`
    // used to cancel that callback and leave the scheduler holding a token the
    // platform can never echo, so later `arm` calls freeze until resume.
    if runtime.scheduler.token_armed() {
        return Ok(false);
    }
    let Some(token) = runtime.scheduler.request() else {
        return Ok(false);
    };
    if !runtime.scheduler.accept(token) {
        return Ok(false);
    }
    let now = crate::net::mono_ms();
    on_frame(
        runtime,
        FrameTime {
            vsync_ms: now,
            target_ms: now,
            interval_ms: 16.0,
            mono_ms: now,
        },
        acks,
    )
}

fn frame(
    runtime: &mut Runtime,
    token: FrameToken,
    time: FrameTime,
    acks: &Acks,
) -> Result<bool, BootError> {
    if !runtime.scheduler.accept(token) {
        return Ok(false);
    }
    on_frame(runtime, time, acks)
}
