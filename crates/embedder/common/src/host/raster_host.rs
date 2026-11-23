//! Graphics state and raster work, owned by the configured raster queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

use engine::session::FrameTime;

use super::{ImageLoader, Installed, elapsed_ms, lock};
use crate::TaskRunner;
use crate::command::SurfaceId;
use crate::diagnostics::TelemetryHub;
use crate::diagnostics::telemetry::{ScheduleTiming, mono_ns, mono_ns_at};
use crate::error::BootError;
use crate::images::{self, ImageCache};
use crate::present::{self, AfterPresent, after_present};
use crate::scheduler::{self, FrameStep, FrameView, Gpu, VsyncOp};
use crate::surface::SurfaceHost;
use crate::task_runner::RunnerSlot;
use crate::timing::FrameTiming;

pub(super) struct RasterHost {
    pub(super) raster: Rc<RefCell<raster::Rasterizer>>,
    pub(super) telemetry: Option<Arc<TelemetryHub>>,
    pub(super) external_textures: HashMap<String, images::ExternalFrame>,
    pub(super) external_gpu: HashMap<String, images::ExternalGpuFrame>,
    pub(super) surfaces: SurfaceHost,
    pub(super) open_surface: crate::OpenSurface,
    pub(super) installed: Option<Installed>,
    pub(super) needs_present: bool,
    pub(super) images: ImageCache,
    pub(super) load_image: ImageLoader,
    pub(super) upload: Arc<Mutex<Option<raster::TextureFactory>>>,
    pub(super) present_failures: u32,
    pub(super) device_recoveries: u8,
    pub(super) gpu: Gpu,
    pub(super) last_step: Option<(FrameStep, Gpu)>,
    pub(super) frame_retry_at: Option<i64>,
    pub(super) clock: Arc<dyn TaskRunner>,
    pub(super) busy: Arc<crate::watchdog::Busy>,
}

impl Drop for RasterHost {
    fn drop(&mut self) {
        self.surfaces.clear();
        *lock(&self.upload) = None;
    }
}

pub(super) enum RasterPort {
    Local(Box<RasterHost>),
    Remote(Arc<RunnerSlot<RasterHost>>),
}

impl RasterPort {
    pub(super) fn with<R: Send + 'static>(
        &mut self,
        work: impl FnOnce(&mut RasterHost) -> R + Send + 'static,
    ) -> Result<R, BootError> {
        match self {
            Self::Local(raster) => Ok(work(raster)),
            Self::Remote(slot) => {
                let slot = Arc::clone(slot);
                let (reply, response) = mpsc::sync_channel(1);
                Arc::clone(slot.runner()).post(Box::new(move || {
                    if let Some(result) = slot.with(work) {
                        let _ = reply.send(result);
                    }
                }));
                response.recv().map_err(|_| BootError::Panicked)
            }
        }
    }

    pub(super) fn post(&mut self, work: impl FnOnce(&mut RasterHost) + Send + 'static) {
        match self {
            Self::Local(raster) => work(raster),
            Self::Remote(slot) => {
                let slot = Arc::clone(slot);
                Arc::clone(slot.runner()).post(Box::new(move || {
                    slot.with(work);
                }));
            }
        }
    }

    pub(super) fn retire(&mut self) {
        if let Self::Remote(slot) = self {
            let slot = Arc::clone(slot);
            let (reply, response) = mpsc::sync_channel(1);
            Arc::clone(slot.runner()).post(Box::new(move || {
                slot.clear();
                let _ = reply.send(());
            }));
            let _ = response.recv();
        }
    }

    #[cfg(test)]
    pub(super) fn local(&mut self) -> &mut RasterHost {
        match self {
            Self::Local(raster) => raster,
            Self::Remote(_) => panic!("test uses a local raster"),
        }
    }
}

pub(super) struct FrameSubmission {
    pub(super) revision: u64,
    pub(super) number: u64,
    pub(super) time: FrameTime,
    pub(super) frame_started: Instant,
    pub(super) schedule: Option<ScheduleTiming>,
    pub(super) telemetry: Option<Arc<TelemetryHub>>,
    pub(super) runtime_sample: Option<engine::session::RuntimeSample>,
    pub(super) session_ms: f64,
    pub(super) gesture_ms: f64,
    pub(super) phases: engine::session::FramePhases,
    pub(super) layout_report: Option<String>,
    pub(super) measure: raster::ShapeStats,
    pub(super) scale: f32,
    pub(super) paused: bool,
    pub(super) again: bool,
    pub(super) tree: Option<Arc<engine::scene::LayerTree>>,
    pub(super) requirements: Arc<[(Arc<str>, bool)]>,
    pub(super) platform_views: Arc<[engine::tree::PlatformViewPlacement]>,
    pub(super) placements_changed: bool,
    pub(super) layers_changed: bool,
    pub(super) reloads: Vec<String>,
    pub(super) census: Option<Arc<engine::semantics::ImageCensus>>,
    pub(super) hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
}

pub(super) struct RasterOutput {
    pub(super) again: bool,
    pub(super) stop: bool,
    pub(super) shown: bool,
    pub(super) timing: Option<FrameTiming>,
    pub(super) gpu_pending: bool,
}

impl RasterOutput {
    fn finish(again: bool) -> Self {
        Self {
            again,
            stop: false,
            shown: false,
            timing: None,
            gpu_pending: false,
        }
    }
}

pub(super) struct RasterCompletion {
    pub(super) submission: FrameSubmission,
    pub(super) output: RasterOutput,
    pub(super) surface: Option<SurfaceId>,
    pub(super) retry_at: Option<i64>,
}

pub(super) fn raster_frame(
    graphics: &mut RasterHost,
    mut submission: FrameSubmission,
    separate: bool,
    before_present: impl FnOnce(),
) -> Result<RasterCompletion, BootError> {
    let _trace = separate.then(|| {
        let phase = engine::session::trace::Section::begin(c"Impasto raster frame");
        engine::session::trace::Section::event(|| {
            format!("Impasto raster frame id={}", submission.number)
        });
        phase
    });
    let span = tracing::error_span!(target: "impasto::render", "raster", frame = submission.number);
    let _entered = span.enter();
    if let Some(schedule) = submission.schedule.as_mut() {
        schedule.raster_start_ns = mono_ns();
    }
    if let Some(hub) = &submission.telemetry {
        drain_gpu(graphics, hub);
    }
    graphics.busy.enter("frame");
    let output = render_layers(graphics, &mut submission, before_present);
    graphics.busy.leave();
    let mut output = output?;
    if let Some(hub) = &submission.telemetry {
        if let Some(timing) = output.timing.as_ref() {
            crate::timing::record_frame(hub, timing, &submission.phases);
        } else if let Some(mut schedule) = submission.schedule {
            schedule.present_done_ns = mono_ns();
            let phase = submission.phases;
            crate::timing::record_frame(
                hub,
                &FrameTiming {
                    number: submission.number,
                    schedule,
                    work_ms: schedule.e2e_ms(),
                    interval_ms: submission.time.interval_ms,
                    session_ms: submission.session_ms,
                    gesture_ms: submission.gesture_ms,
                    script_ms: phase.script_ms,
                    layout_ms: phase.layout_ms,
                    record_ms: phase.record_ms,
                    runtime: submission.runtime_sample,
                    ..FrameTiming::default()
                },
                &phase,
            );
        }
        output.gpu_pending = graphics.raster.borrow().gpu_timings_pending();
    }
    Ok(RasterCompletion {
        submission,
        output,
        surface: graphics.surfaces.id(),
        retry_at: graphics.frame_retry_at,
    })
}

fn render_layers(
    graphics: &mut RasterHost,
    submission: &mut FrameSubmission,
    before_present: impl FnOnce(),
) -> Result<RasterOutput, BootError> {
    use engine::session::trace::Section;

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
    let reloads = std::mem::take(&mut submission.reloads);
    let census = submission.census.take();
    let reloaded = !reloads.is_empty();
    for source in reloads {
        graphics.images.reload(&source);
        graphics.raster.borrow_mut().forget_texture(&source);
        graphics.installed = None;
    }
    again |= reloaded;
    let Some(tree) = submission.tree.as_ref() else {
        return Ok(RasterOutput::finish(again));
    };
    if !images_ready(graphics, &submission.requirements, scale, census.as_deref()) {
        return Ok(RasterOutput::finish(reloaded));
    }
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
        FrameStep::Cancel | FrameStep::Skip { again: false } => {
            let _trace = Section::begin(c"Impasto skip present");
            return Ok(RasterOutput::finish(false));
        }
        FrameStep::Skip { again: true } => {
            let _trace = Section::begin(c"Impasto skip present");
            return Ok(RasterOutput::finish(true));
        }
        FrameStep::Submit => {
            let _trace = Section::begin(c"Impasto submit");
            let bucket = images::scale_bucket(scale);
            if graphics
                .installed
                .as_ref()
                .is_some_and(|installed| images::scale_bucket(installed.scale) != bucket)
            {
                graphics.raster.borrow_mut().invalidate_textures();
            }
            graphics.images.install_uploaded(
                &mut graphics.raster.borrow_mut(),
                images::scale_bucket(scale),
            );
            {
                let mut raster = graphics.raster.borrow_mut();
                for (source, frame) in &graphics.external_gpu {
                    if let Some(image) = &frame.image {
                        raster.install_uploaded(source, image);
                    }
                }
            }
            let started = Instant::now();
            let pixels = graphics
                .images
                .snapshot_requirements(&submission.requirements, images::scale_bucket(scale));
            let resolve = |source: &str| {
                if source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX) {
                    return graphics.external_textures.get(source).map(|frame| {
                        raster::ImagePixels {
                            width: frame.size[0],
                            height: frame.size[1],
                            rgba: &frame.pixels,
                        }
                    });
                }
                pixels
                    .iter()
                    .find(|(name, ..)| name == source)
                    .map(|(_, width, height, rgba)| raster::ImagePixels {
                        width: *width,
                        height: *height,
                        rgba,
                    })
            };
            let submitted = graphics
                .raster
                .borrow_mut()
                .submit_layers(tree, scale, resolve);
            submit_ms = elapsed_ms(started);
            if let Err(status) = submitted {
                if status == raster::Status::DeviceLost {
                    recover_device(graphics)?;
                    return Ok(RasterOutput::finish(true));
                }
                tracing::error!(
                    target: "impasto::render::present",
                    %status,
                    "scene submit failed"
                );
                if let Some(hub) = submission.hub.as_ref() {
                    hub.error(
                        "engine",
                        &format!("scene submit failed: {status}"),
                        None,
                        Some(number),
                    );
                }
                // A rejected list is not a lost surface. Do not force another vsync.
                return Ok(RasterOutput::finish(again));
            }
            graphics.installed = Some(Installed { scale });
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
    let gpu_frame = submission
        .telemetry
        .as_ref()
        .filter(|hub| hub.collects_gpu())
        .map(|_| number);
    let presented = {
        let _trace = Section::begin(c"Impasto present");
        let mut raster = graphics.raster.borrow_mut();
        raster.begin_gpu_frame(gpu_frame);
        graphics
            .surfaces
            .present(&mut raster, &submission.platform_views)
    };
    let present_done = Instant::now();
    let present_ms = present_done.duration_since(started).as_secs_f64() * 1000.0;
    let work_ms = present_done.duration_since(frame_started).as_secs_f64() * 1000.0;
    let (mut stats, mut gpu) = {
        let raster = graphics.raster.borrow();
        (raster.submit_stats(), raster.gpu_sample())
    };
    if !matches!(step, FrameStep::Submit) {
        stats.picture_prepare = std::time::Duration::ZERO;
        stats.picture_rebuilt = 0;
        stats.picture_reused = 0;
        stats.image_upload = raster::ImageUploadStats::default();
    }
    if !presented.is_ok() && stats.submit_done.is_none() {
        stats.render = raster::RenderStats::default();
        stats.atlas = raster::AtlasStats::default();
        stats.compose = std::time::Duration::ZERO;
        stats.encode = std::time::Duration::ZERO;
        stats.flush = std::time::Duration::ZERO;
        stats.queue_submit = std::time::Duration::ZERO;
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
            "render resources");
    }
    if matches!(step, FrameStep::Submit) {
        Section::event(|| {
            format!(
                "Impasto pictures rebuilt={} reused={} layers={} prepare_us={} compose_us={}",
                stats.picture_rebuilt,
                stats.picture_reused,
                stats.layer_count,
                stats.picture_prepare.as_micros(),
                stats.compose.as_micros(),
            )
        });
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
    let mut output = RasterOutput {
        again,
        stop: false,
        shown: presented.is_ok(),
        timing: Some(timing),
        gpu_pending: false,
    };
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
    let after = after_present(presented, again);
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

pub(super) fn drain_gpu(graphics: &mut RasterHost, hub: &TelemetryHub) -> bool {
    let mut raster = graphics.raster.borrow_mut();
    if raster.gpu_timings_pending() {
        for update in raster.drain_gpu_timings() {
            hub.patch_gpu_timing(update);
        }
    }
    raster.gpu_timings_pending()
}

fn recover_device(graphics: &mut RasterHost) -> Result<(), BootError> {
    if graphics.device_recoveries >= 2 {
        return Err(BootError::Engine(raster::Status::DeviceLost));
    }
    if let Some(hub) = &graphics.telemetry {
        for update in graphics.raster.borrow_mut().retire_gpu_timings() {
            hub.patch_gpu_timing(update);
        }
    }
    graphics.device_recoveries += 1;
    tracing::warn!(target: "impasto::render::gpu", attempts = graphics.device_recoveries, "graphics device lost, rebuilding");
    *lock(&graphics.upload) = None;
    graphics.images.invalidate_gpu();
    let mut raster = graphics.raster.borrow_mut();
    graphics
        .surfaces
        .recover_device(&mut raster)
        .map_err(BootError::Engine)?;
    for frame in graphics.external_gpu.values_mut() {
        frame.image = (frame.import)(&raster);
    }
    *lock(&graphics.upload) = raster.texture_factory();
    graphics.installed = None;
    graphics.gpu = Gpu::Flush;
    graphics.needs_present = true;
    Ok(())
}

fn images_ready(
    graphics: &mut RasterHost,
    requirements: &[(Arc<str>, bool)],
    scale: f32,
    census: Option<&engine::semantics::ImageCensus>,
) -> bool {
    graphics.images.begin_frame();
    if let Some(census) = census {
        graphics.images.sync_counts(census.counts.as_ref());
    }
    let bucket = images::scale_bucket(scale);
    graphics.images.pin_sources(
        requirements.iter().map(|(source, _)| source.as_ref()),
        bucket,
    );
    let mut waiting = false;
    let mut blocked = false;
    for (source, required) in requirements {
        let source = source.as_ref();
        let required = *required;
        if source.starts_with(engine::scene::image::EXTERNAL_TEXTURE_PREFIX) {
            continue;
        }
        if graphics.images.is_pending(source, bucket) {
            if required {
                waiting = true;
            }
            continue;
        }
        if graphics.images.resolve(source, bucket).is_some() {
            continue;
        }
        if graphics.images.is_negative(source, bucket) {
            if required {
                blocked = true;
                if let Some(message) = graphics.images.take_required_log(source, bucket) {
                    tracing::error!(target: "impasto::render::image", "{message}");
                }
            }
            continue;
        }
        if let Some(request) = graphics.images.request(source, bucket)
            && !(graphics.load_image)(request)
        {
            tracing::error!(target: "impasto::render::image", "image decoder stopped");
        }
        if required {
            waiting = true;
        }
    }
    if blocked {
        return false;
    }
    !waiting
}

fn note_step(runtime: &mut RasterHost, step: FrameStep) {
    let gpu = runtime.gpu;
    if runtime.last_step == Some((step, gpu)) {
        return;
    }
    runtime.last_step = Some((step, gpu));
    tracing::debug!(target: "impasto::render", ?step, ?gpu, "frame step");
}
