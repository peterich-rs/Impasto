use super::{UiHost, arm, flush_gpu};
use crate::command::Command;
use crate::error::BootError;
use crate::images::ImageReady;

pub(super) fn handle(runtime: &mut UiHost, command: Command) -> Result<bool, BootError> {
    match command {
        Command::Invalidate => invalidate(runtime),
        Command::MemoryWarning => memory_warning(runtime),
        Command::ImageReady(ready) => image_ready(runtime, ready),
        Command::ExternalTexture { source, frame } => external_texture(runtime, source, frame),
        Command::ExternalGpuTexture { source, frame } => {
            external_gpu_texture(runtime, source, frame)
        }
        Command::ReloadImage(source) => reload(runtime, source),
        _ => {}
    }
    Ok(false)
}

fn invalidate(runtime: &mut UiHost) {
    tracing::debug!(target: "impasto::render::gpu", "invalidate");
    runtime.graphics.post(|graphics| {
        graphics.needs_present = true;
        graphics.frame_retry_at = None;
    });
    runtime.present_retry_at = None;
    arm(runtime);
}

fn memory_warning(runtime: &mut UiHost) {
    flush_gpu(runtime, "memory");
    runtime.graphics.post(|graphics| graphics.images.trim());
    if !runtime.input.is_paused() {
        arm(runtime);
    }
}

fn image_ready(runtime: &mut UiHost, ready: ImageReady) {
    let decoded =
        matches!(&ready, ImageReady::Decoded(handle) if uses_texture(runtime, &handle.source));
    runtime.graphics.post(move |graphics| {
        let request_id = match &ready {
            ImageReady::Decoded(handle) => handle.generation,
            ImageReady::Missing { generation, .. } | ImageReady::Failed { generation, .. } => {
                *generation
            }
        };
        tracing::event!(name: "image_cache_apply", target: "impasto::io", tracing::Level::DEBUG,
            trace_event = true, queue = "raster", request_id);
        if decoded {
            graphics.installed = None;
        }
        if let Some(message) = graphics.images.apply(ready) {
            tracing::error!(target: "impasto::render::image", "{message}");
        }
    });
    arm(runtime);
}

fn reload(runtime: &mut UiHost, source: String) {
    tracing::debug!(target: "impasto::render::image", source, "image reload");
    runtime.graphics.post(move |graphics| {
        graphics.images.reload(&source);
        graphics.raster.borrow_mut().forget_texture(&source);
        graphics.installed = None;
    });
    arm(runtime);
}

fn external_texture(runtime: &mut UiHost, source: String, frame: Option<crate::ExternalFrame>) {
    let used = uses_texture(runtime, &source);
    runtime.graphics.post(move |graphics| {
        graphics.external_gpu.remove(&source);
        if let Some(frame) = frame {
            graphics.external_textures.insert(source.clone(), frame);
        } else {
            graphics.external_textures.remove(&source);
        }
        texture_updated(graphics, &source, used);
    });
    if used && !runtime.input.is_paused() {
        arm(runtime);
    }
}

fn external_gpu_texture(
    runtime: &mut UiHost,
    source: String,
    frame: Option<crate::ImportExternalFrame>,
) {
    let used = uses_texture(runtime, &source);
    runtime.graphics.post(move |graphics| {
        graphics.external_textures.remove(&source);
        if let Some(import) = frame {
            let image = import(&graphics.raster.borrow());
            graphics.external_gpu.insert(
                source.clone(),
                crate::images::ExternalGpuFrame { import, image },
            );
        } else {
            graphics.external_gpu.remove(&source);
        }
        texture_updated(graphics, &source, used);
    });
    if used && !runtime.input.is_paused() {
        arm(runtime);
    }
}

fn uses_texture(runtime: &UiHost, source: &str) -> bool {
    runtime
        .image_requirements
        .iter()
        .any(|(key, _)| key.as_ref() == source)
}

fn texture_updated(graphics: &mut super::RasterHost, source: &str, used: bool) {
    graphics.raster.borrow_mut().forget_texture(source);
    if used {
        graphics.installed = None;
    }
}
