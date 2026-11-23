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
use crate::error::BootError;
use crate::images::{self, ImageCache};
use crate::present::{self, AfterPresent, after_present};
use crate::scheduler::{self, FrameStep, FrameView, Gpu, VsyncOp};
use crate::surface::SurfaceHost;
use crate::task_runner::RunnerSlot;
use crate::timing::FrameTiming;

pub(super) struct RasterHost {
    pub(super) raster: Rc<RefCell<raster::Rasterizer>>,
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
}

impl RasterOutput {
    fn finish(again: bool) -> Self {
        Self {
            again,
            stop: false,
            shown: false,
            timing: None,
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
    graphics.busy.enter("frame");
    let output = render_layers(graphics, &mut submission, before_present);
    graphics.busy.leave();
    Ok(RasterCompletion {
        submission,
        output: output?,
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
    let presented = {
        let _trace = Section::begin(c"Impasto present");
        let mut raster = graphics.raster.borrow_mut();
        graphics
            .surfaces
            .present(&mut raster, &submission.platform_views)
    };
    let present_ms = elapsed_ms(started);
    let work_ms = elapsed_ms(frame_started);
    let stats = graphics.raster.borrow().submit_stats();
    if presented.is_ok() {
        tracing::trace!(target: "impasto::render::gpu",
            draws = stats.render.draws,
            passes = stats.render.passes,
            pooled_texture_bytes = stats.render.pooled_texture_bytes,
            buffers_created = stats.render.buffers_created,
            bind_groups_created = stats.render.bind_groups_created,
            instance_bytes = stats.render.instance_bytes,
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
    let timing = FrameTiming {
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
    };
    let present_error = presented.as_ref().err().copied();
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

fn recover_device(graphics: &mut RasterHost) -> Result<(), BootError> {
    if graphics.device_recoveries >= 2 {
        return Err(BootError::Engine(raster::Status::DeviceLost));
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
