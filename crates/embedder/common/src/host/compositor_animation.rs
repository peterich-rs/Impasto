//! Routes one display-link callback at a time to an idle UI's compositor.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use engine::scene::{CompositorProp, CompositorSnapshot, CompositorTrack};
use engine::session::FrameTime;

use super::{
    FrameSubmission, RasterCompletion, RasterHost, UiHost, end_frame, lock, ms_to_ns,
    next_frame_number, raster_frame,
    raster_host::{images_ready, install_scene},
    refresh_wake,
};
use crate::FrameRateHint;
use crate::command::{Command, FrameToken, SurfaceId};
use crate::diagnostics::telemetry::{ScheduleTiming, mono_ns_at};
use crate::error::BootError;
use crate::queue::Inbox;
use crate::scheduler::Gpu;
use crate::task_runner::RunnerSlot;

// Keep callback tokens positive in platforms that transport them as signed longs.
const RASTER_TOKEN: u64 = 1 << 62;

/// The callback routing word is zero on UI and holds the expected token on Raster.
#[derive(Default)]
pub(super) struct CompositorMode {
    expected: AtomicU64,
    next: AtomicU64,
    active: Mutex<Option<Active>>,
    callback_gate: Mutex<()>,
}

struct Active {
    generation: u64,
    plan: Arc<Plan>,
    latest: Option<Arc<CompositorSnapshot>>,
    last_vsync: Option<FrameTime>,
    window: ListWindow,
    surface: Option<SurfaceId>,
    retry_at: Option<i64>,
    requested_rate: FrameRateHint,
    candidate_rate: FrameRateHint,
    rate_frames: u8,
}

enum ListWindow {
    Idle,
    Preparing,
    Ready(Box<FrameSubmission>),
}

struct Plan {
    tree: Arc<engine::scene::LayerTree>,
    revision: u64,
    requirements: Arc<[(Arc<str>, bool)]>,
    scale: f32,
    topology: &'static str,
    vsync: Option<Arc<dyn crate::Vsync>>,
    telemetry: Option<Arc<crate::diagnostics::TelemetryHub>>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
}

impl Active {
    fn frame_rate(&mut self, target_ms: f64) -> FrameRateHint {
        let hint = frame_rate(&self.plan.tree.tracks, target_ms);
        if hint != self.candidate_rate {
            self.candidate_rate = hint;
            self.rate_frames = 0;
        }
        self.rate_frames = self.rate_frames.saturating_add(1);
        if hint != self.requested_rate && (hint == FrameRateHint::Display || self.rate_frames >= 3)
        {
            self.requested_rate = hint;
            tracing::event!(
                name: "compositor_cadence",
                target: "impasto::compositor",
                tracing::Level::DEBUG,
                trace_event = true,
                cadence = ?hint
            );
        }
        self.requested_rate
    }
}

fn frame_rate(tracks: &[CompositorTrack], target_ms: f64) -> FrameRateHint {
    if tracks.is_empty() {
        return FrameRateHint::Display;
    }
    let mut rate = FrameRateHint::Hz30;
    for track in tracks {
        let velocity = track.speed(target_ms);
        match track.prop {
            CompositorProp::ScrollX | CompositorProp::ScrollY if velocity < 80.0 => {
                rate = FrameRateHint::Hz60;
            }
            CompositorProp::Opacity if velocity.is_finite() => {
                if velocity > 90.0 {
                    rate = FrameRateHint::Hz60;
                }
            }
            _ => return FrameRateHint::Display,
        }
    }
    rate
}

impl CompositorMode {
    pub(super) fn is_raster_token(token: FrameToken) -> bool {
        token.0 & RASTER_TOKEN != 0
    }

    fn next_token(&self) -> FrameToken {
        FrameToken(RASTER_TOKEN | (self.next.fetch_add(1, Ordering::Relaxed) & (RASTER_TOKEN - 1)))
    }

    fn begin(&self, plan: Plan, surface: Option<SurfaceId>) -> FrameToken {
        let token = self.next_token();
        *lock(&self.active) = Some(Active {
            generation: token.0,
            plan: Arc::new(plan),
            latest: None,
            last_vsync: None,
            window: ListWindow::Idle,
            surface,
            retry_at: None,
            requested_rate: FrameRateHint::Display,
            candidate_rate: FrameRateHint::Display,
            rate_frames: 0,
        });
        self.expected.store(token.0, Ordering::Release);
        token
    }

    fn expects(&self, token: FrameToken) -> bool {
        self.expected.load(Ordering::Acquire) == token.0
    }

    pub(super) fn cancel(&self, vsync: &dyn crate::Vsync) {
        let _gate = lock(&self.callback_gate);
        let token = self.expected.swap(0, Ordering::AcqRel);
        if token != 0 {
            vsync.cancel_if_token(FrameToken(token));
        }
    }

    fn cancel_callback(&self, token: FrameToken) -> bool {
        self.expected
            .compare_exchange(token.0, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn continue_callback(&self, token: FrameToken) -> Option<FrameToken> {
        let next = self.next_token();
        self.expected
            .compare_exchange(token.0, next.0, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(next)
    }

    pub(super) fn route(
        self: &Arc<Self>,
        slot: &Arc<RunnerSlot<RasterHost>>,
        inbox: &Arc<Inbox>,
        token: FrameToken,
        time: FrameTime,
    ) {
        if !self.expects(token) {
            return;
        }
        let posted_at = Instant::now();
        let mode = Arc::clone(self);
        let slot = Arc::clone(slot);
        let result_inbox = Arc::clone(inbox);
        let posted = Arc::clone(slot.runner()).post(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                slot.with(|graphics| mode.frame(graphics, &result_inbox, token, time, posted_at))
                    .unwrap_or(Err(BootError::Panicked))
            }))
            .unwrap_or(Err(BootError::Panicked));
            if let Err(error) = result {
                result_inbox.complete(Box::new(move |_| Err(error)));
            }
        }));
        if !posted && self.cancel_callback(token) {
            inbox.complete(Box::new(|_| Err(BootError::Panicked)));
        }
    }

    fn frame(
        &self,
        graphics: &mut RasterHost,
        inbox: &Inbox,
        token: FrameToken,
        time: FrameTime,
        posted_at: Instant,
    ) -> Result<(), BootError> {
        if !self.expects(token) {
            return Ok(());
        }
        let Some((generation, mut plan)) = lock(&self.active)
            .as_ref()
            .map(|run| (run.generation, Arc::clone(&run.plan)))
        else {
            return Ok(());
        };
        if graphics.gpu != Gpu::Live || !graphics.surfaces.is_open() || graphics.installed.is_none()
        {
            self.handoff(inbox, token, generation, None);
            return Ok(());
        }
        let installed_window = self.install_window(graphics, token, generation, time)?;
        let snapshot = {
            let mut raster = graphics.raster.borrow_mut();
            if let Some(window) = installed_window {
                plan = window;
                raster.sample_compositor_window(plan.tree.epoch, time.target_ms)
            } else {
                raster.sample_compositor_tracks(plan.tree.epoch, time.target_ms)
            }
        };
        let Some(snapshot) = snapshot else {
            if plan
                .tree
                .tracks
                .iter()
                .any(|track| track.crosses_scroll_threshold(time.target_ms))
            {
                tracing::event!(name: "compositor_threshold_handoff", target: "impasto::compositor",
                    tracing::Level::DEBUG, trace_event = true, queue = "raster",
                    ui_woken = true, target_ms = time.target_ms);
            }
            self.handoff(inbox, token, generation, Some(time));
            return Ok(());
        };
        if !self.expects(token) {
            return Ok(());
        }
        if plan
            .tree
            .tracks
            .iter()
            .any(|track| track.needs_list_prefetch(time.target_ms, time.interval_ms))
        {
            self.request_window(inbox, generation, time);
        }
        let submission = plan.submission(time, posted_at);
        let span = tracing::info_span!(target: "impasto::compositor", "compositor_frame",
            frame_id = submission.number, queue = "raster", active_tracks = plan.tree.tracks.len(),
            ui_woken = false, target_ms = time.target_ms, missed_vsync = tracing::field::Empty,
        )
        .entered();
        graphics.needs_present = true;
        let completion = raster_frame(graphics, submission, true, || {})?;
        let missed =
            time.interval_ms > 0.0 && posted_at.elapsed().as_secs_f64() * 1000.0 > time.interval_ms;
        span.record("missed_vsync", missed);
        let output = completion.output;
        let mut rate = FrameRateHint::Display;
        if let Some(run) = lock(&self.active).as_mut()
            && run.generation == generation
        {
            run.surface = completion.surface;
            run.retry_at = completion.retry_at;
            if output.shown {
                run.last_vsync = Some(time);
                rate = run.frame_rate(time.target_ms);
                if !snapshot.samples.is_empty() {
                    run.latest = Some(Arc::new(snapshot));
                }
            }
        }
        if output.stop {
            self.cancel_callback(token);
            inbox.complete(Box::new(|runtime| {
                runtime.services.borrow().request_exit();
                Ok(end_frame(runtime, false, true))
            }));
        } else if output.shown {
            let _gate = lock(&self.callback_gate);
            if let Some(next) = self.continue_callback(token)
                && let Some(vsync) = &plan.vsync
            {
                vsync.request_frame_with_rate(next, rate);
            }
        } else {
            self.handoff(inbox, token, generation, None);
        }
        Ok(())
    }

    fn request_window(&self, inbox: &Inbox, generation: u64, time: FrameTime) {
        let requested = {
            let mut active = lock(&self.active);
            active.as_mut().is_some_and(|run| {
                if run.generation != generation || !matches!(run.window, ListWindow::Idle) {
                    return false;
                }
                run.window = ListWindow::Preparing;
                true
            })
        };
        if requested {
            tracing::event!(name: "compositor_list_prefetch", target: "impasto::compositor",
                tracing::Level::DEBUG, trace_event = true, queue = "raster",
                ui_woken = true, target_ms = time.target_ms);
            inbox.post(Command::PrefetchCompositorWindow { generation, time });
        }
    }

    pub(super) fn running(&self) -> bool {
        self.expected.load(Ordering::Acquire) != 0
    }

    pub(super) fn prepare_content(&self, generation: u64) -> bool {
        let mut active = lock(&self.active);
        let Some(run) = active.as_mut().filter(|run| run.generation == generation) else {
            return false;
        };
        if !matches!(run.window, ListWindow::Idle) {
            return false;
        }
        run.window = ListWindow::Preparing;
        true
    }

    /// Publish one UI-built window for installation at the next compositor vsync.
    pub(super) fn publish_window(
        &self,
        generation: u64,
        epoch: u64,
        submission: FrameSubmission,
    ) -> bool {
        let mut active = lock(&self.active);
        let Some(run) = active.as_mut().filter(|run| run.generation == generation) else {
            return false;
        };
        if !matches!(run.window, ListWindow::Preparing) {
            return false;
        }
        run.window = ListWindow::Idle;
        if submission.compositor_epoch != Some(epoch)
            || submission.revision != run.plan.revision
            || submission.placements_changed
            || !submission.reloads.is_empty()
            || !self.running()
            || submission.tree.as_ref().is_none_or(|tree| {
                tree.tracks.len() != run.plan.tree.tracks.len()
                    || tree
                        .tracks
                        .iter()
                        .any(|track| !run.plan.tree.tracks.iter().any(|old| old.id == track.id))
            })
        {
            return false;
        }
        if submission.layers_changed {
            run.window = ListWindow::Ready(Box::new(submission));
        }
        true
    }

    fn install_window(
        &self,
        graphics: &mut RasterHost,
        token: FrameToken,
        generation: u64,
        time: FrameTime,
    ) -> Result<Option<Arc<Plan>>, BootError> {
        let Some((old_plan, mut submission)) = lock(&self.active)
            .as_mut()
            .filter(|run| {
                run.generation == generation && matches!(run.window, ListWindow::Ready(_))
            })
            .and_then(|run| {
                if let ListWindow::Ready(pending) =
                    std::mem::replace(&mut run.window, ListWindow::Idle)
                {
                    Some((Arc::clone(&run.plan), pending))
                } else {
                    None
                }
            })
        else {
            return Ok(None);
        };
        let Some(tree) = submission.tree.clone() else {
            return Ok(None);
        };
        if !tree
            .tracks
            .iter()
            .all(|track| track.sample(time.target_ms).is_some())
        {
            return Ok(None);
        }
        graphics
            .images
            .set_layout_targets(tree.image_decode_extents(), submission.scale);
        if !images_ready(
            graphics,
            &submission.requirements,
            submission.scale,
            submission.census.take().as_deref(),
        ) {
            if let Some(run) = lock(&self.active).as_mut()
                && run.generation == generation
            {
                run.window = ListWindow::Ready(submission);
            }
            return Ok(None);
        }
        if !self.expects(token) || install_scene(graphics, &submission)?.0.is_none() {
            return Ok(None);
        }
        let plan = Arc::new(Plan {
            tree,
            revision: submission.revision,
            scale: submission.scale,
            requirements: submission.requirements,
            topology: old_plan.topology,
            vsync: old_plan.vsync.clone(),
            telemetry: old_plan.telemetry.clone(),
            hub: old_plan.hub.clone(),
        });
        if let Some(run) = lock(&self.active).as_mut()
            && run.generation == generation
        {
            run.plan = Arc::clone(&plan);
        }
        Ok(Some(plan))
    }

    fn handoff(&self, inbox: &Inbox, token: FrameToken, generation: u64, time: Option<FrameTime>) {
        if self.cancel_callback(token) {
            let snapshot = lock(&self.active)
                .as_ref()
                .and_then(|run| run.latest.clone());
            inbox.post(Command::CommitCompositorState {
                generation,
                snapshot,
                time,
            });
        }
    }
}

/// Allows the next raster sample to request another UI turn while rows settle.
pub(super) fn defer_window(runtime: &UiHost, generation: u64) {
    if let Some(run) = lock(&runtime.compositor_mode.active).as_mut()
        && run.generation == generation
        && matches!(run.window, ListWindow::Preparing)
    {
        run.window = ListWindow::Idle;
    }
}

/// A newly decoded asset for a pending window can be installed without stopping the old one.
pub(super) fn prefetch_image(runtime: &UiHost, source: &str) -> bool {
    let active = lock(&runtime.compositor_mode.active);
    active.as_ref().is_some_and(|run| {
        runtime.compositor_generation == Some(run.generation)
            && !run
                .plan
                .requirements
                .iter()
                .any(|(key, _)| key.as_ref() == source)
    })
}

impl Plan {
    fn submission(&self, time: FrameTime, frame_started: Instant) -> FrameSubmission {
        let telemetry = self.telemetry.clone().filter(|hub| hub.collects_frames());
        let posted_ns = mono_ns_at(frame_started);
        let schedule = telemetry.as_ref().map(|_| ScheduleTiming {
            vsync_ns: Some(posted_ns),
            ui_start_ns: posted_ns,
            ui_done_ns: posted_ns,
            topology: Some(self.topology),
            budget_ns: ms_to_ns(time.interval_ms),
            compositor_only: true,
            ..ScheduleTiming::default()
        });
        FrameSubmission {
            capture_frame: telemetry
                .as_ref()
                .and_then(|hub| hub.track_frame(posted_ns)),
            sample_gpu: telemetry.as_ref().is_some_and(|hub| hub.collects_gpu()),
            revision: self.revision,
            surface: None,
            interaction: None,
            number: next_frame_number(),
            time,
            frame_started,
            schedule,
            telemetry,
            runtime_sample: None,
            session_ms: 0.0,
            gesture_ms: 0.0,
            phases: Default::default(),
            layout_report: None,
            measure: Default::default(),
            scale: self.scale,
            paused: false,
            again: true,
            tree: Some(Arc::clone(&self.tree)),
            scene_delta: None,
            requirements: Arc::default(),
            platform_views: Arc::default(),
            placements_changed: false,
            layers_changed: false,
            reloads: Vec::new(),
            census: None,
            hub: self.hub.clone(),
            compositor_epoch: None,
            compositor_frame: true,
            animated_effects: false,
            gpu_views: false,
        }
    }
}

/// Give Raster display-link ownership only after all preceding UI frames are installed.
pub(super) fn activate(runtime: &mut UiHost, completion: &RasterCompletion) -> bool {
    let submission = &completion.submission;
    if submission.compositor_epoch != Some(runtime.compositor_epoch) {
        return false;
    }
    if !completion.output.shown
        || !completion.output.again
        || completion.retry_at.is_some()
        || submission.revision != runtime.revision
        || runtime.compositor_generation.is_some()
        || runtime.frames.in_flight != 0
        || runtime.frames.pending.is_some()
        || runtime.input.is_paused()
        || runtime.inbox.has_pending()
    {
        tracing::event!(name: "compositor_deferred", target: "impasto::compositor",
            tracing::Level::DEBUG, trace_event = true, queue = "ui",
            status = "queued_work", shown = completion.output.shown,
            in_flight = runtime.frames.in_flight, armed = runtime.scheduler.token_armed(),
            pending = runtime.inbox.has_pending());
        return false;
    }
    let Some(tree) = submission.tree.clone() else {
        return false;
    };
    if tree.tracks.is_empty() && !completion.output.renderer_work {
        return false;
    }
    // A preceding UI frame may have prearmed the next tick while Raster
    // was encoding. Retire that token before transferring vsync ownership.
    if runtime.scheduler.token_armed() {
        runtime.cancel_vsync();
    }
    let token = runtime.compositor_mode.begin(
        Plan {
            tree,
            revision: submission.revision,
            requirements: Arc::clone(&submission.requirements),
            scale: submission.scale,
            topology: runtime.queue_placement,
            vsync: Some(Arc::clone(&runtime.vsync)),
            telemetry: runtime.telemetry.clone(),
            hub: runtime.hub.clone(),
        },
        completion.surface,
    );
    runtime.compositor_generation = Some(token.0);
    refresh_wake(runtime);
    runtime.vsync.request_frame(token);
    true
}

/// Cancel callbacks and commit the latest completed sample without waiting for Raster.
pub(super) fn stop(runtime: &mut UiHost, announced: Option<Arc<CompositorSnapshot>>) -> bool {
    let Some(generation) = runtime.compositor_generation.take() else {
        return false;
    };
    runtime.compositor_mode.cancel(runtime.vsync.as_ref());
    let active = lock(&runtime.compositor_mode.active)
        .take()
        .filter(|active| active.generation == generation);
    let latest = active.and_then(|active| {
        runtime.active_surface = active.surface;
        runtime.present_retry_at = active.retry_at;
        if let Some(last_vsync) = active.last_vsync {
            runtime.last_vsync = Some(last_vsync);
        }
        if !active.plan.tree.tracks.is_empty() {
            // The raster queue invalidates its sampled tree before the next UI submission.
            runtime.raster_dirty = true;
            runtime.graphics.post(|graphics| graphics.installed = None);
        }
        active.latest
    });
    let snapshot = latest.or(announced);
    let _span = tracing::info_span!(target: "impasto::compositor", "anim_sync",
        queue = "ui", active_tracks = snapshot.as_ref().map_or(0, |snapshot| snapshot.samples.len()),
        ui_woken = true,
    ).entered();
    if let Some(snapshot) = snapshot
        && let Some(session) = runtime.session.as_mut()
    {
        session.commit_compositor_state(&snapshot);
    }
    true
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use engine::scene::LayerTree;

    use super::{CompositorMode, ListWindow, Plan};

    fn plan() -> Plan {
        Plan {
            tree: Arc::new(LayerTree::empty()),
            revision: 0,
            requirements: Arc::default(),
            scale: 1.0,
            topology: "platform-ui",
            vsync: None,
            telemetry: None,
            hub: None,
        }
    }

    #[test]
    fn publishing_a_new_window_keeps_raster_vsync_ownership() {
        use engine::session::FrameTime;
        use std::time::Instant;

        let mode = CompositorMode::default();
        let first = mode.begin(plan(), None);
        let make_window = || {
            let mut next = plan().submission(FrameTime::at(100.0), Instant::now());
            next.layers_changed = true;
            next.compositor_epoch = Some(1);
            next
        };
        let preparing =
            || super::lock(&mode.active).as_mut().unwrap().window = ListWindow::Preparing;
        preparing();
        assert!(!mode.publish_window(first.0 + 1, 1, make_window()));
        assert!(!mode.publish_window(first.0, 2, make_window()));
        preparing();
        assert!(mode.publish_window(first.0, 1, make_window()));
        assert!(mode.expects(first));
        assert!(matches!(
            super::lock(&mode.active).as_ref().unwrap().window,
            ListWindow::Ready(_)
        ));
        let next = mode.continue_callback(first).unwrap();
        assert!(mode.cancel_callback(next));
        assert!(!mode.publish_window(first.0, 1, make_window()));
    }

    #[test]
    fn stale_and_cancelled_callbacks_cannot_rearm_raster() {
        let mode = CompositorMode::default();
        let first = mode.begin(plan(), None);
        assert!(CompositorMode::is_raster_token(first));
        assert!(mode.expects(first));
        let second = mode.continue_callback(first).expect("next raster callback");
        assert_ne!(first, second);
        assert!(!mode.expects(first));
        assert!(mode.cancel_callback(second));
        assert!(!mode.expects(second));
        assert!(mode.continue_callback(second).is_none());

        let resumed = mode.begin(plan(), None);
        assert_ne!(resumed, second);
        assert!(!mode.cancel_callback(second));
        assert!(mode.expects(resumed));
    }
}
