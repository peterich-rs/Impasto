//! Graphics state and raster work, owned by the configured raster queue.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use engine::session::FrameTime;

use super::{ImageLoader, Installed, elapsed_ms, lock};
use crate::TaskRunner;
use crate::command::SurfaceId;
use crate::diagnostics::telemetry::{ScheduleTiming, mono_ns};
use crate::diagnostics::{CaptureFrame, TelemetryHub};
use crate::error::BootError;
use crate::images::{self, ImageCache};
use crate::scheduler::{FrameStep, Gpu};
use crate::surface::SurfaceHost;
use crate::task_runner::RunnerSlot;
use crate::timing::FrameTiming;

mod frame;

use frame::render_layers;

pub(super) struct RasterHost {
    pub(super) raster: Rc<RefCell<raster::Rasterizer>>,
    pub(super) telemetry: Option<Arc<TelemetryHub>>,
    pub(super) gpu_poll_pending: bool,
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
    pub(super) capture_frame: Option<CaptureFrame>,
    pub(super) sample_gpu: bool,
    pub(super) revision: u64,
    pub(super) surface: Option<SurfaceId>,
    pub(super) interaction: Option<u64>,
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
    pub(super) scene_delta: Option<engine::scene::SceneDelta>,
    pub(super) requirements: Arc<[(Arc<str>, bool)]>,
    pub(super) platform_views: Arc<[engine::tree::PlatformViewPlacement]>,
    pub(super) placements_changed: bool,
    pub(super) layers_changed: bool,
    pub(super) reloads: Vec<String>,
    pub(super) census: Option<Arc<engine::semantics::ImageCensus>>,
    pub(super) hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
    pub(super) compositor_epoch: Option<u64>,
    pub(super) compositor_frame: bool,
    pub(super) animated_effects: bool,
    pub(super) gpu_views: bool,
}

pub(super) struct RasterOutput {
    pub(super) again: bool,
    pub(super) stop: bool,
    pub(super) shown: bool,
    pub(super) timing: Option<FrameTiming>,
    pub(super) gpu_pending: bool,
    pub(super) renderer_work: bool,
}

impl RasterOutput {
    fn finish(again: bool) -> Self {
        Self {
            again,
            stop: false,
            shown: false,
            timing: None,
            gpu_pending: false,
            renderer_work: false,
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
    _separate: bool,
    before_present: impl FnOnce(),
) -> Result<RasterCompletion, BootError> {
    let capture_frame = submission.capture_frame.take();
    let span = tracing::info_span!(
        target: "impasto::render",
        "raster",
        frame = submission.number,
        surface = graphics.surfaces.id().map(|id| id.0),
        interaction = submission.interaction,
        queue = "raster"
    );
    let entered = span.enter();
    tracing::debug!(target: "impasto::render", id = submission.number, "Impasto raster");
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
    drop(entered);
    drop(capture_frame);
    Ok(RasterCompletion {
        submission,
        output,
        surface: graphics.surfaces.id(),
        retry_at: graphics.frame_retry_at,
    })
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

/// Profiling must complete on Raster in split hosts, including between UI turns.
pub(super) fn schedule_gpu_poll(slot: Arc<RunnerSlot<RasterHost>>, hub: Arc<TelemetryHub>) {
    Arc::clone(slot.runner()).post(Box::new(move || {
        let armed = slot.with(|graphics| {
            if graphics.gpu_poll_pending || !graphics.raster.borrow().gpu_timings_pending() {
                return false;
            }
            graphics.gpu_poll_pending = true;
            true
        });
        if armed == Some(true) {
            poll_gpu_later(&slot, hub, 8);
        }
    }));
}

fn poll_gpu_later(slot: &Arc<RunnerSlot<RasterHost>>, hub: Arc<TelemetryHub>, delay_ms: u64) {
    let pending_slot = Arc::downgrade(slot);
    slot.runner().post_delayed(
        Duration::from_millis(delay_ms),
        Box::new(move || {
            let Some(slot) = pending_slot.upgrade() else {
                return;
            };
            let pending = slot.with(|graphics| {
                let pending = drain_gpu(graphics, &hub);
                graphics.gpu_poll_pending = pending;
                pending
            });
            if pending == Some(true) {
                poll_gpu_later(&slot, hub, (delay_ms * 2).min(64));
            }
        }),
    );
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

pub(super) fn install_scene(
    graphics: &mut RasterHost,
    submission: &FrameSubmission,
) -> Result<(Option<raster::SubmitInfo>, f64), BootError> {
    let Some(tree) = submission.tree.as_ref() else {
        return Ok((None, 0.0));
    };
    let FrameSubmission { scale, number, .. } = *submission;
    let _trace = tracing::debug_span!(target: "impasto::render", "submit").entered();
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
            return graphics
                .external_textures
                .get(source)
                .map(|frame| raster::ImagePixels {
                    width: frame.size[0],
                    height: frame.size[1],
                    rgba: &frame.pixels,
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
    let submitted = graphics.raster.borrow_mut().submit_scene(
        Arc::clone(tree),
        scale,
        submission.scene_delta.as_ref(),
        resolve,
    );
    let submit_ms = elapsed_ms(started);
    let submitted = match submitted {
        Ok(submitted) => submitted,
        Err(status) => {
            if status == raster::Status::DeviceLost {
                recover_device(graphics)?;
                return Ok((None, submit_ms));
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
            return Ok((None, submit_ms));
        }
    };
    graphics.installed = Some(Installed { scale });
    Ok((Some(submitted), submit_ms))
}

pub(super) fn images_ready(
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
        let ready = graphics.images.is_ready(source, bucket);
        if graphics.images.is_pending(source, bucket) {
            waiting |= required && !ready;
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
        waiting |= required && !ready;
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
