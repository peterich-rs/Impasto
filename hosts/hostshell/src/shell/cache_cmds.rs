use super::{Runtime, arm};
use crate::LogLevel;
use crate::command::Command;
use crate::error::BootError;
use crate::images::ImageReady;

pub(super) fn handle(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Invalidate => invalidate(runtime),
        Command::MemoryWarning => memory_warning(runtime),
        Command::ImageReady(ready) => image_ready(runtime, ready),
        Command::ReloadImage(source) => reload(runtime, &source),
        _ => {}
    }
    Ok(false)
}

fn invalidate(runtime: &mut Runtime) {
    runtime.needs_present = true;
    runtime.present_retry_at = None;
    arm(runtime);
}

fn memory_warning(runtime: &mut Runtime) {
    runtime.installed = None;
    runtime.present_retry_at = None;
    runtime.images.trim();
    arm(runtime);
}

fn image_ready(runtime: &mut Runtime, ready: ImageReady) {
    if let Some(message) = runtime.images.apply(ready) {
        (runtime.log)(LogLevel::Error, &message);
    }
    arm(runtime);
}

fn reload(runtime: &mut Runtime, source: &str) {
    runtime.images.reload(source);
    arm(runtime);
}
