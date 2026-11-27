use std::sync::Arc;

use engine::session::Metrics;

use super::{Acks, RasterHost, UiHost, arm, note_scale};
use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::error::BootError;
use crate::surface::{Changed, Destroyed, OpenRetry, PendingMetrics, Ready};

pub(super) fn handle(
    runtime: &mut UiHost,
    command: Command,
    acks: &Arc<Acks>,
) -> Result<bool, BootError> {
    match command {
        Command::SurfaceDestroyed { id } => destroyed(runtime, id, acks),
        Command::SurfaceReady(target) => ready(runtime, target, acks),
        Command::SurfaceChanged {
            id,
            metrics,
            pixels,
        } => changed(runtime, id, metrics, pixels),
        _ => Ok(false),
    }
}

fn destroyed(runtime: &mut UiHost, id: SurfaceId, acks: &Arc<Acks>) -> Result<bool, BootError> {
    let dropped = runtime
        .graphics
        .with(move |graphics| retire(graphics, id))?;
    did_retire(runtime, id, dropped);
    acks.signal(id);
    Ok(false)
}

pub(super) fn retire(graphics: &mut RasterHost, id: SurfaceId) -> bool {
    let _retire = tracing::info_span!(
        target: "impasto::render::surface",
        "surface_retire",
        surface = id.0,
        queue = "raster",
    )
    .entered();
    graphics.surfaces.drop_pending(id);
    if graphics.surfaces.destroyed(id) == Destroyed::Dropped {
        graphics.raster.borrow_mut().set_resource_viewport(None);
        graphics.installed = None;
        graphics.needs_present = crate::scheduler::surface_lost().needs_present;
        graphics.frame_retry_at = None;
        true
    } else {
        false
    }
}

pub(super) fn did_retire(runtime: &mut UiHost, id: SurfaceId, _dropped: bool) {
    if runtime.active_surface == Some(id) {
        tracing::info!(target: "impasto::render::surface", surface = id.0, "surface destroyed");
        runtime.active_surface = None;
        runtime.present_retry_at = None;
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.cancel_vsync();
    }
}

enum OpenOutcome {
    Stale,
    Opened {
        id: SurfaceId,
        metrics: Metrics,
        replaced: Option<SurfaceId>,
    },
    Retry {
        id: SurfaceId,
        attempts: u32,
        retry: OpenRetry,
    },
    Failed {
        id: SurfaceId,
        status: raster::Status,
    },
}

fn ready(runtime: &mut UiHost, target: SurfaceTarget, acks: &Arc<Acks>) -> Result<bool, BootError> {
    if !runtime.inbox.admits_surface(target.id) {
        return Ok(false);
    }
    let acks = Arc::clone(acks);
    runtime.update_raster_checked(
        move |graphics| {
            let attempts = graphics.surfaces.take_prior_attempts(target.id);
            open(graphics, target, attempts)
        },
        move |runtime, result| apply(runtime, result, &acks),
    )?;
    Ok(false)
}

pub(super) fn retry(runtime: &mut UiHost, acks: &Arc<Acks>) -> Result<(), BootError> {
    runtime.try_surface = false;
    let acks = Arc::clone(acks);
    runtime.update_raster_checked(
        |graphics| {
            graphics
                .surfaces
                .take_attempt()
                .map_or(OpenOutcome::Stale, |(target, attempts)| {
                    open(graphics, target, attempts)
                })
        },
        move |runtime, result| apply(runtime, result, &acks),
    )
}

fn open(graphics: &mut RasterHost, target: SurfaceTarget, attempts: u32) -> OpenOutcome {
    let id = target.id;
    let metrics = target.metrics;
    let pixels = target.pixels;
    let kept = SurfaceTarget {
        id,
        metrics,
        pixels,
        layer: target.layer,
    };
    let mut raster = graphics.raster.borrow_mut();
    let _span = tracing::info_span!(target: "impasto::render::surface", "surface_open",
        surface = id.0, attempts)
    .entered();
    match graphics
        .surfaces
        .ready(target, &mut raster, graphics.open_surface.as_mut())
    {
        Ready::Opened { replaced, .. } => {
            raster.set_resource_viewport(None);
            raster.set_resource_viewport(Some(pixels));
            for frame in graphics.external_gpu.values_mut() {
                if frame.image.is_none() {
                    frame.image = (frame.import)(&raster);
                }
            }
            *super::lock(&graphics.upload) = raster.texture_factory();
            graphics.needs_present = true;
            graphics.installed = None;
            OpenOutcome::Opened {
                id,
                metrics,
                replaced,
            }
        }
        Ready::Failed(
            raster::Status::NotReady | raster::Status::SurfaceLost | raster::Status::AcquireMiss,
        ) => {
            let retry = graphics.surfaces.fail_attempt(kept, attempts);
            OpenOutcome::Retry {
                id,
                attempts,
                retry,
            }
        }
        Ready::Failed(status) => OpenOutcome::Failed { id, status },
        Ready::Stale => OpenOutcome::Stale,
    }
}

fn apply(runtime: &mut UiHost, result: OpenOutcome, acks: &Arc<Acks>) -> Result<(), BootError> {
    let id = match result {
        OpenOutcome::Opened { id, .. }
        | OpenOutcome::Retry { id, .. }
        | OpenOutcome::Failed { id, .. } => id,
        OpenOutcome::Stale => return Ok(()),
    };
    if !runtime.inbox.admits_surface(id)
        || runtime.active_surface.is_some_and(|active| active.0 > id.0)
    {
        return Ok(());
    }
    match result {
        OpenOutcome::Opened {
            metrics, replaced, ..
        } => {
            if let Some(old) = replaced {
                acks.signal(old);
            }
            runtime.services.borrow().surface_attached(id);
            runtime.active_surface = Some(id);
            runtime.try_surface = false;
            runtime.revision = runtime.revision.wrapping_add(1);
            note_scale(runtime, metrics.device_pixel_ratio);
            runtime.input.set_metrics(metrics);
            arm(runtime);
            tracing::info!(target: "impasto::render::surface", surface = id.0, "surface opened");
        }
        OpenOutcome::Retry {
            attempts, retry, ..
        } => {
            runtime.try_surface = matches!(retry, OpenRetry::Again);
            if runtime.try_surface {
                tracing::warn!(target: "impasto::render::surface", surface = id.0, attempts, "surface open not ready");
                arm(runtime);
            } else {
                tracing::error!(target: "impasto::render::surface", surface = id.0, "surface open failed; stopping retry");
            }
        }
        OpenOutcome::Failed { status, .. } => return Err(BootError::Engine(status)),
        OpenOutcome::Stale => {}
    }
    Ok(())
}

fn changed(
    runtime: &mut UiHost,
    id: SurfaceId,
    metrics: Metrics,
    pixels: raster::PixelSize,
) -> Result<bool, BootError> {
    runtime.update_raster(
        move |graphics| {
            let changed = graphics.surfaces.changed(id, metrics, pixels);
            let pending = if let Changed::Staged { resized } = changed {
                graphics.needs_present |= resized;
                if resized {
                    graphics
                        .raster
                        .borrow_mut()
                        .set_resource_viewport(Some(pixels));
                }
                PendingMetrics::Absent
            } else {
                graphics.surfaces.retarget(id, metrics, pixels)
            };
            (changed, pending)
        },
        move |runtime, (changed, pending)| match (changed, pending) {
            (Changed::Staged { resized }, _) if runtime.active_surface == Some(id) => {
                note_scale(runtime, metrics.device_pixel_ratio);
                let metrics_changed = runtime.input.set_metrics(metrics);
                if metrics_changed || (resized && !runtime.input.is_paused()) {
                    arm(runtime);
                }
            }
            (_, PendingMetrics::Updated) => note_scale(runtime, metrics.device_pixel_ratio),
            (_, PendingMetrics::Rearm) => {
                runtime.try_surface = true;
                note_scale(runtime, metrics.device_pixel_ratio);
                arm(runtime);
            }
            _ => {}
        },
    );
    Ok(false)
}
