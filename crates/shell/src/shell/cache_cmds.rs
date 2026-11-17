use super::{Runtime, arm, flush_gpu};
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
    tracing::debug!(target: "impasto::render::gpu", "invalidate");
    runtime.needs_present = true;
    runtime.present_retry_at = None;
    arm(runtime);
}

fn memory_warning(runtime: &mut Runtime) {
    flush_gpu(runtime, "memory");
    runtime.images.trim();
    if !runtime.input.is_paused() {
        arm(runtime);
    }
}

fn image_ready(runtime: &mut Runtime, ready: ImageReady) {
    if let Some(message) = runtime.images.apply(ready) {
        tracing::error!(target: "impasto::render::image", "{message}");
    }
    arm(runtime);
}

fn reload(runtime: &mut Runtime, source: &str) {
    tracing::debug!(target: "impasto::render::image", source, "image reload");
    runtime.images.reload(source);
    arm(runtime);
}
