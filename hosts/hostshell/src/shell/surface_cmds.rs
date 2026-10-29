use session::Metrics;

use super::{Acks, OpenPlace, OpenedSurface, Runtime, arm, note_scale, open_surface_target};
use crate::command::{Command, SurfaceId, SurfaceTarget};
use crate::error::BootError;
use crate::surface::{Changed, Destroyed, PendingMetrics};

pub(super) fn handle(
    runtime: &mut Runtime,
    command: Command,
    acks: &Acks,
) -> Result<bool, BootError> {
    match command {
        Command::SurfaceDestroyed { id } => destroyed(runtime, id, acks),
        Command::SurfaceReady(target) => ready(runtime, target, acks),
        Command::SurfaceChanged { id, metrics } => changed(runtime, id, metrics),
        _ => Ok(false),
    }
}

fn destroyed(runtime: &mut Runtime, id: SurfaceId, acks: &Acks) -> Result<bool, BootError> {
    runtime.surfaces.drop_pending(id);
    if runtime.surfaces.destroyed(id) == Destroyed::Dropped {
        let follow = crate::scheduler::surface_lost();
        runtime.installed = None;
        runtime.needs_present = follow.needs_present;
        runtime.scheduler.cancel();
        runtime.vsync.cancel();
    }
    acks.signal(id);
    Ok(false)
}

fn ready(runtime: &mut Runtime, target: SurfaceTarget, acks: &Acks) -> Result<bool, BootError> {
    let attempts = runtime.surfaces.take_prior_attempts(target.id);
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

fn changed(runtime: &mut Runtime, id: SurfaceId, metrics: Metrics) -> Result<bool, BootError> {
    if runtime.surfaces.changed(id, metrics) == Changed::Staged {
        note_scale(runtime, metrics.device_pixel_ratio);
        if runtime.input.set_metrics(metrics) {
            arm(runtime);
        }
    } else {
        match runtime.surfaces.retarget(id, metrics) {
            PendingMetrics::Absent => {}
            PendingMetrics::Updated => note_scale(runtime, metrics.device_pixel_ratio),
            PendingMetrics::Rearm => {
                note_scale(runtime, metrics.device_pixel_ratio);
                arm(runtime);
            }
        }
    }
    Ok(false)
}
