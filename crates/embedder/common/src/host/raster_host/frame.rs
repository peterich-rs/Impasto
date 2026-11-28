//! Scene admission, rendering, and presentation on the raster queue.

use std::time::{Duration, Instant};

use super::{
    FrameSubmission, RasterHost, RasterOutput, images_ready, install_scene, note_step,
    recover_device,
};
use crate::Presented;
use crate::diagnostics::telemetry::{ScheduleTiming, mono_ns_at};
use crate::error::BootError;
use crate::present::{self, AfterPresent, after_present};
use crate::scheduler::{self, FrameStep, FrameView, Gpu, VsyncOp};
use crate::timing::FrameTiming;

// The resource callback wakes the host immediately. This deadline also covers
// a job that stops without publishing a resource for the current scene.
const RESOURCE_RETRY_MS: i64 = 16;
const MAX_RESOURCE_RETRY_MS: i64 = 128;

pub(super) fn render_layers(
    graphics: &mut RasterHost,
    submission: &mut FrameSubmission,
    before_present: impl FnOnce(),
) -> Result<RasterOutput, BootError> {
    let FrameSubmission {
        scale,
        number,
        time,
        frame_started,
        session_ms,
        gesture_ms,
        phases: phase,
        layers_changed,
        mut again,
        ..
    } = *submission;
    if !graphics.surfaces.is_open()
        || submission
            .surface
            .is_some_and(|id| graphics.surfaces.id() != Some(id))
    {
        return Ok(RasterOutput::finish(false));
    }
    let reloads = std::mem::take(&mut submission.reloads);
    let census = submission.census.take();
    let reloaded = apply_reloads(graphics, reloads);
    again |= reloaded;
    let Some(tree) = submission.tree.as_ref() else {
        return Ok(RasterOutput::finish(again));
    };
    let prepare_deadline = Duration::try_from_secs_f64(time.interval_ms / 1_000.0)
        .ok()
        .filter(|interval| !interval.is_zero())
        .and_then(|interval| {
            frame_started.checked_add(interval.saturating_sub(Duration::from_millis(2)))
        });
    graphics
        .raster
        .borrow_mut()
        .set_resource_prepare_deadline(prepare_deadline);
    let scene_delta = submission.scene_delta.as_ref().filter(|_| {
        layers_changed
            && !reloaded
            && census.is_none()
            && graphics.installed.is_some()
            && graphics.gpu == Gpu::Live
            && graphics.surfaces.is_open()
            && !submission.paused
    });
    let incremental = if let Some(delta) = scene_delta {
        graphics
            .raster
            .borrow_mut()
            .submit_layer_delta(tree, scale, delta)
            .ok()
            .flatten()
    } else {
        None
    };
    if !submission.compositor_frame && incremental.is_none() {
        if layers_changed
            || graphics
                .installed
                .as_ref()
                .is_none_or(|installed| installed.scale != scale)
        {
            graphics
                .images
                .set_layout_targets(tree.image_decode_extents(), scale);
        }
        if !images_ready(graphics, &submission.requirements, scale, census.as_deref()) {
            return Ok(RasterOutput::finish(reloaded));
        }
    }
    let mut renderer_work = submission.animated_effects
        || (submission.gpu_views && graphics.raster.borrow().has_registered_gpu_views_in_scene());
    again |= renderer_work;
    graphics.needs_present |= renderer_work;
    graphics.needs_present |= submission.placements_changed;
    if graphics
        .frame_retry_at
        .is_some_and(|deadline| crate::timing::ms_i64(graphics.clock.monotonic_ms()) < deadline)
    {
        return Ok(RasterOutput::finish(again));
    }
    let unchanged = !layers_changed
        && graphics
            .installed
            .as_ref()
            .is_some_and(|installed| installed.scale == scale);
    graphics.needs_present |= graphics.raster.borrow_mut().collect_resources();
    let mut submit_ms = 0.0;
    let step = scheduler::frame_step(&FrameView {
        surface_open: graphics.surfaces.is_open(),
        paused: submission.paused,
        unchanged,
        needs_present: graphics.needs_present,
        again,
        gpu: graphics.gpu,
    });
    note_step(graphics, step);
    if again && matches!(step, FrameStep::Submit | FrameStep::PresentCached) {
        before_present();
    }
    match step {
        FrameStep::Cancel | FrameStep::Skip { .. } => {
            let status = if matches!(step, FrameStep::Cancel) {
                "cancelled"
            } else {
                "unchanged"
            };
            tracing::event!(name: "present_skipped", target: "impasto::render", tracing::Level::DEBUG,
                trace_event = true, status);
            return Ok(RasterOutput::finish(matches!(
                step,
                FrameStep::Skip { again: true }
            )));
        }
        FrameStep::Submit => {
            let (submitted, elapsed) = if let Some(incremental) = incremental {
                let elapsed = incremental.stats.picture_prepare.as_secs_f64() * 1_000.0;
                (Some(incremental), elapsed)
            } else {
                install_scene(graphics, submission)?
            };
            submit_ms = elapsed;
            let Some(submitted) = submitted else {
                return Ok(RasterOutput::finish(again || graphics.gpu == Gpu::Flush));
            };
            renderer_work = submission.animated_effects
                || (submission.gpu_views
                    && graphics.raster.borrow().has_registered_gpu_views_in_scene());
            again |= renderer_work;
            if submitted.stats.damage == raster::DamageRect::Empty
                && graphics.gpu == Gpu::Live
                && !graphics.needs_present
            {
                tracing::event!(name: "present_skipped", target: "impasto::render", tracing::Level::DEBUG,
                    trace_event = true, status = "no_damage");
                return Ok(RasterOutput::finish(again));
            }
            if graphics.gpu == Gpu::Flush {
                graphics.gpu = Gpu::Live;
                tracing::info!(target: "impasto::render::gpu", reason = "resume", "gpu live");
            }
        }
        FrameStep::PresentCached => {}
    }
    if graphics.gpu != Gpu::Live {
        return Ok(RasterOutput::finish(false));
    }
    let started = Instant::now();
    let present_start_ns = submission.schedule.map(|_| mono_ns_at(started));
    let gpu_frame = submission.sample_gpu.then_some(number);
    let presented = {
        let _trace = tracing::debug_span!(target: "impasto::render", "present").entered();
        let mut raster = graphics.raster.borrow_mut();
        raster.set_render_time_ms(submission.time.target_ms);
        raster.begin_gpu_frame(gpu_frame);
        let result = graphics
            .surfaces
            .present(&mut raster, &submission.platform_views);
        for failure in raster.take_effect_failures() {
            if let Some(hub) = &submission.hub {
                let message = format!("Shader {}: {}", failure.shader.0, failure.reason);
                hub.error("runtimeEffect", &message, None, Some(number));
            }
        }
        result
    };
    let present_done = Instant::now();
    let status = if presented.is_ok() {
        "present_proxy"
    } else {
        "failed"
    };
    tracing::event!(name: "present_outcome", target: "impasto::render", tracing::Level::DEBUG,
        trace_event = true, status,
        reason = presented.as_ref().err().map(ToString::to_string).as_deref());
    let present_ms = present_done.duration_since(started).as_secs_f64() * 1000.0;
    let work_ms = present_done.duration_since(frame_started).as_secs_f64() * 1000.0;
    let (mut stats, mut gpu) = {
        let raster = graphics.raster.borrow();
        (raster.submit_stats(), raster.gpu_sample())
    };
    if !matches!(step, FrameStep::Submit) {
        stats.picture_prepare = Duration::ZERO;
        stats.glyph_prepare = Duration::ZERO;
        stats.picture_rebuilt = 0;
        stats.picture_reused = 0;
        stats.image_upload = raster::ImageUploadStats::default();
    }
    if !presented.is_ok() && stats.submit_done.is_none() {
        stats.render = raster::RenderStats::default();
        stats.atlas = raster::AtlasStats::default();
        stats.compose = Duration::ZERO;
        stats.encode = Duration::ZERO;
        stats.flush = Duration::ZERO;
        stats.queue_submit = Duration::ZERO;
    }
    if presented.is_ok() {
        tracing::trace!(target: "impasto::render::gpu",
            draws = stats.render.draws,
            passes = stats.render.passes,
            pooled_texture_bytes = stats.render.pooled_texture_bytes,
            buffers_created = stats.render.buffers_created,
            bind_groups_created = stats.render.bind_groups_created,
            vertex_bytes = stats.render.vertex_bytes,
            uniform_bytes = stats.render.uniform_bytes,
            stop_bytes = stats.render.stop_bytes,
            offscreen_passes = stats.render.offscreen_passes,
            direct_present = stats.render.direct_present,
            atlas_bytes = stats.atlas.bytes,
            atlas_hits = stats.atlas.hits,
            atlas_misses = stats.atlas.misses,
            atlas_uploads = stats.atlas.uploads,
            atlas_upload_calls = stats.atlas.upload_calls,
            atlas_evictions = stats.atlas.evictions,
            mask_raster_us = u64::try_from(stats.atlas.mask_raster.as_micros()).unwrap_or(u64::MAX),
            "render resources");
    }
    if matches!(step, FrameStep::Submit) {
        tracing::debug!(
            target: "impasto::render",
            rebuilt = stats.picture_rebuilt,
            reused = stats.picture_reused,
            layers = stats.layer_count,
            prepare_us = u64::try_from(stats.picture_prepare.as_micros()).unwrap_or(u64::MAX),
            glyph_pre_us = u64::try_from(stats.glyph_prepare.as_micros()).unwrap_or(u64::MAX),
            compose_us = u64::try_from(stats.compose.as_micros()).unwrap_or(u64::MAX),
            "pictures_submitted"
        );
    }
    let shapes = submission.measure;
    let present_error = presented.as_ref().err().copied();
    if present_error == Some(raster::Status::DeviceLost)
        && gpu.status == raster::GpuTimingStatus::Pending
    {
        gpu.status = raster::GpuTimingStatus::StaleDevice;
    }
    let schedule = submission
        .schedule
        .map_or_else(ScheduleTiming::default, |mut schedule| {
            schedule.present_start_ns = present_start_ns;
            schedule.submit_done_ns = stats.submit_done.map(mono_ns_at);
            schedule.present_done_ns = mono_ns_at(present_done);
            schedule.missed_vsync = schedule.budget_ns > 0
                && schedule
                    .present_done_ns
                    .saturating_sub(schedule.vsync_ns.unwrap_or(schedule.ui_start_ns))
                    > schedule.budget_ns;
            schedule
        });
    let timing = FrameTiming {
        schedule,
        raster: stats,
        gpu,
        runtime: submission.runtime_sample,
        number,
        vsync_ms: time.vsync_ms,
        target_ms: time.target_ms,
        interval_ms: time.interval_ms,
        work_ms,
        session_ms,
        gesture_ms,
        script_ms: phase.script_ms,
        layout_ms: phase.layout_ms,
        layout_cb_ms: phase.layout_cb_ms,
        sync_ms: phase.sync_ms,
        record_ms: phase.record_ms,
        semantics_ms: phase.semantics_ms,
        submit_ms,
        present_ms,
        presented: matches!(presented, Ok(Presented::Continue)),
        present_error,
        shapes_built: shapes.built,
        shapes_measure_hit: shapes.measure_hit,
        shapes_measure_miss: shapes.measure_miss,
        shapes_paint_hit: shapes.paint_hit,
        shapes_paint_miss: shapes.paint_miss,
        atlas_hits: stats.atlas.hits,
        picture_rebuilt: stats.picture_rebuilt,
        picture_reused: stats.picture_reused,
        layer_count: stats.layer_count,
        atlas_entries: u32::try_from(stats.atlas.entries).unwrap_or(u32::MAX),
        pictures: stats.pictures,
        textures: stats.textures,
    };
    let output = RasterOutput {
        again,
        stop: false,
        shown: presented.is_ok(),
        timing: Some(timing),
        gpu_pending: false,
        renderer_work,
    };
    if let Some(hub) = &submission.telemetry {
        crate::timing::record_frame(hub, &timing, &phase);
    }
    finish_present(graphics, submission, output, presented, present_error)
}

fn apply_reloads(graphics: &mut RasterHost, reloads: Vec<String>) -> bool {
    let reloaded = !reloads.is_empty();
    for source in reloads {
        graphics.images.reload(&source);
        graphics.raster.borrow_mut().forget_texture(&source);
        graphics.installed = None;
    }
    reloaded
}

fn finish_present(
    graphics: &mut RasterHost,
    submission: &FrameSubmission,
    mut output: RasterOutput,
    presented: Result<Presented, raster::Status>,
    present_error: Option<raster::Status>,
) -> Result<RasterOutput, BootError> {
    if present_error == Some(raster::Status::ResourcesPending) {
        let delay = graphics
            .resource_retry_delay_ms
            .map_or(RESOURCE_RETRY_MS, |previous| {
                (previous * 2).min(MAX_RESOURCE_RETRY_MS)
            });
        graphics.resource_retry_delay_ms = Some(delay);
        graphics.needs_present = true;
        graphics.frame_retry_at =
            Some(crate::timing::ms_i64(graphics.clock.monotonic_ms()).saturating_add(delay));
        output.again = false;
        return Ok(output);
    }
    graphics.resource_retry_delay_ms = None;
    let number = submission.number;
    if present_error == Some(raster::Status::DeviceLost) {
        recover_device(graphics)?;
        return Ok(RasterOutput {
            again: true,
            ..output
        });
    }
    if present_error == Some(raster::Status::OutOfMemory) {
        graphics.raster.borrow_mut().trim();
        graphics.images.trim();
        graphics.installed = None;
    }
    let after = after_present(presented, output.again);
    if after == AfterPresent::Fatal {
        return Err(BootError::Engine(
            present_error.unwrap_or(raster::Status::DeviceLost),
        ));
    }
    if after == AfterPresent::Rejected {
        graphics.needs_present = false;
        return Ok(output);
    }
    let follow = scheduler::present_followup(after);
    graphics.needs_present = follow.needs_present;
    if follow.clear_surface {
        graphics.surfaces.clear();
        graphics.installed = None;
    }
    if matches!(after, AfterPresent::Shown { .. } | AfterPresent::Stop) {
        if matches!(after, AfterPresent::Shown { .. }) {
            graphics.busy.arm();
        }
        if graphics.present_failures >= present::PRESENT_BACKOFF_AFTER {
            tracing::info!(target: "impasto::render::present", "present recovered");
        }
        graphics.present_failures = 0;
        graphics.device_recoveries = 0;
        graphics.frame_retry_at = None;
    }
    let mut again = follow.vsync == VsyncOp::Request;
    if after == AfterPresent::Retry {
        graphics.present_failures = graphics.present_failures.saturating_add(1);
        if let Some(delay) = present::present_retry_delay_ms(graphics.present_failures) {
            if graphics.present_failures == present::PRESENT_BACKOFF_AFTER {
                tracing::warn!(
                    target: "impasto::render::present",
                    failures = graphics.present_failures,
                    retry_in_ms = delay,
                    "present failed; backing off"
                );
                if let Some(hub) = submission.hub.as_ref() {
                    hub.error("engine", "present failed; backing off", None, Some(number));
                }
            }
            graphics.frame_retry_at =
                Some(crate::timing::ms_i64(graphics.clock.monotonic_ms()).saturating_add(delay));
            again = false;
        } else {
            graphics.frame_retry_at = None;
        }
    }
    output.stop = follow.vsync == VsyncOp::Stop;
    if output.stop {
        graphics.gpu = Gpu::Dead;
    }
    output.again = again;
    Ok(output)
}
