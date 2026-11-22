use super::{Runtime, arm, flush_gpu};
use crate::command::Command;
use crate::error::BootError;
use crate::images::ImageReady;

pub(super) fn handle(runtime: &mut Runtime, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Invalidate => invalidate(runtime),
        Command::MemoryWarning => memory_warning(runtime),
        Command::ImageReady(ready) => image_ready(runtime, ready),
        Command::ExternalTexture { source, frame } => external_texture(runtime, source, frame),
        Command::ExternalGpuTexture { source, frame } => {
            external_gpu_texture(runtime, source, frame)
        }
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

fn external_texture(runtime: &mut Runtime, source: String, frame: Option<crate::ExternalFrame>) {
    if let Some(frame) = frame {
        runtime.external_textures.insert(source.clone(), frame);
    } else {
        runtime.external_textures.remove(&source);
    }
    texture_updated(runtime, &source);
}

fn external_gpu_texture(
    runtime: &mut Runtime,
    source: String,
    frame: Option<crate::ImportExternalFrame>,
) {
    if let Some(import) = frame {
        let image = import(&runtime.raster.borrow());
        if image.is_none() {
            return;
        }
        runtime.external_gpu.insert(
            source.clone(),
            crate::images::ExternalGpuFrame { import, image },
        );
    } else {
        runtime.external_gpu.remove(&source);
    }
    texture_updated(runtime, &source);
}

fn texture_updated(runtime: &mut Runtime, source: &str) {
    runtime.raster.borrow_mut().forget_texture(source);
    if runtime
        .image_requirements
        .iter()
        .any(|(key, _)| key.as_ref() == source)
    {
        runtime.installed = None;
        if !runtime.input.is_paused() {
            arm(runtime);
        }
    }
}
