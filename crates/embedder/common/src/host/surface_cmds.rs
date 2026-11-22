use engine::session::Metrics;

use super::{Acks, OpenPlace, OpenedSurface, UiHost, arm, note_scale, open_surface_target};
use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::error::BootError;
use crate::surface::{Changed, Destroyed, PendingMetrics};

pub(super) fn handle(
    runtime: &mut UiHost,
    command: Command,
    acks: &Acks,
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

fn destroyed(runtime: &mut UiHost, id: SurfaceId, acks: &Acks) -> Result<bool, BootError> {
    let dropped = runtime
        .graphics
        .with(move |graphics| retire(graphics, id))?;
    did_retire(runtime, id, dropped);
    acks.signal(id);
    Ok(false)
}

pub(super) fn retire(graphics: &mut super::RasterHost, id: SurfaceId) -> bool {
    graphics.surfaces.drop_pending(id);
    if graphics.surfaces.destroyed(id) == Destroyed::Dropped {
        graphics.installed = None;
        graphics.needs_present = crate::scheduler::surface_lost().needs_present;
        graphics.frame_retry_at = None;
        true
    } else {
        false
    }
}

pub(super) fn did_retire(runtime: &mut UiHost, id: SurfaceId, dropped: bool) {
    if dropped || runtime.active_surface == Some(id) {
        tracing::info!(target: "impasto::render::surface", surface = id.0, "surface destroyed");
        runtime.active_surface = None;
        runtime.present_retry_at = None;
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.scheduler.cancel();
        runtime.vsync.cancel();
    }
}

fn ready(runtime: &mut UiHost, target: SurfaceTarget, acks: &Acks) -> Result<bool, BootError> {
    let id = target.id;
    let attempts = runtime
        .graphics
        .with(move |graphics| graphics.surfaces.take_prior_attempts(id))?;
    match open_surface_target(runtime, target, attempts, OpenPlace::Command)? {
        OpenedSurface::Ready { replaced } => {
            if let Some(old) = replaced {
                acks.signal(old);
            }
        }
        OpenedSurface::Waiting => {}
    }
    Ok(false)
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
