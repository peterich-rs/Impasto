//! UI frame admission, input delivery, and handoff to the raster queue.

use std::sync::Arc;
use std::time::Instant;

use engine::scene::SceneDelta;
use engine::session::{FrameOutput, FramePhases, FrameTime, RuntimeSample};

use super::{
    Acks, FramePipeline, FrameSubmission, Inbox, RasterCompletion, RasterHost, RasterPort,
    RunnerSlot, UiHost, arm, compositor_animation, elapsed_ms, end_frame, fault_followup,
    layer_tree_line, ms_to_ns, next_frame_number, raster_frame, refresh_wake, session_call,
    surface_cmds,
};
use super::{issue_list, schedule_gpu_poll, write_inspect};
use crate::diagnostics::telemetry::{ScheduleTiming, mono_ns, mono_ns_at};
use crate::error::BootError;
use crate::input::FrameInput;

pub(super) fn on_frame(
    runtime: &mut UiHost,
    time: FrameTime,
    from_vsync: bool,
    prefetch_generation: Option<u64>,
    acks: &Arc<Acks>,
) -> Result<bool, BootError> {
    let frame_started = Instant::now();
    let counters = runtime
        .telemetry
        .as_ref()
        .is_some_and(|hub| hub.collects_frames());
    let mut schedule = frame_schedule(runtime, time, frame_started, counters, from_vsync);
    let raster_admitted =
        prefetch_generation.is_some() || runtime.frames.has_room(FramePipeline::DEPTH);
    if !raster_admitted && runtime.input.is_paused() {
        note_admission_failure(runtime, schedule, time.interval_ms, "queue_full");
        return Ok(end_frame(runtime, false, false));
    }
    if prefetch_generation.is_none() && runtime.try_surface {
        surface_cmds::retry(runtime, acks)?;
    }
    if runtime.active_surface.is_none() {
        note_admission_failure(runtime, schedule, time.interval_ms, "no_surface");
        return Ok(end_frame(runtime, false, false));
    }
    runtime.frame_number = next_frame_number();
    let capture_frame = schedule.and_then(|schedule| {
        runtime
            .telemetry
            .as_ref()?
            .track_frame(schedule.ui_start_ns)
    });
    let frame_span = tracing::info_span!(
        target: "impasto::frame",
        "frame",
        frame = runtime.frame_number,
        surface = runtime.active_surface.map(|id| id.0),
        interaction = runtime.interaction,
        queue = "ui",
        topology = runtime.queue_placement,
        vsync_ms = time.vsync_ms,
        target_ms = time.target_ms
    );
    let entered = frame_span.enter();
    if from_vsync {
        let vsync_us = ms_to_ns(time.mono_ms) / 1_000;
        tracing::debug!(target: "impasto::frame", id = runtime.frame_number, vsync_us, "Impasto frame");
    } else {
        tracing::debug!(target: "impasto::frame", id = runtime.frame_number, "Impasto frame");
    }
    #[cfg(any(debug_assertions, feature = "devtools-server"))]
    if let Some(session) = runtime.session.as_mut() {
        session.begin_repaint_frame(
            runtime
                .telemetry
                .as_deref()
                .is_some_and(crate::diagnostics::TelemetryHub::tracks_repaints),
            runtime.frame_number,
        );
    }
    let (paused, gesture_ms) = deliver_frame_input(runtime);
    let session_started = Instant::now();
    let Some(SessionFrame {
        again,
        leave,
        reloads,
        census,
        layers_changed,
        scene_delta,
        placements_changed,
        phases,
        runtime_sample,
        layout_report,
        compositor_epoch,
        prefetch_wait,
    }) = run_session(
        runtime,
        time,
        counters,
        paused,
        prefetch_generation,
        &mut schedule,
    )
    else {
        return Ok(end_frame(runtime, false, false));
    };
    let mut submission = FrameSubmission {
        capture_frame,
        sample_gpu: runtime
            .telemetry
            .as_ref()
            .is_some_and(|hub| hub.collects_gpu()),
        revision: runtime.revision,
        surface: runtime.active_surface,
        interaction: runtime.interaction,
        number: runtime.frame_number,
        time,
        frame_started,
        schedule,
        telemetry: runtime.telemetry.clone().filter(|_| counters),
        runtime_sample,
        session_ms: elapsed_ms(session_started),
        gesture_ms,
        phases: phases.unwrap_or_default(),
        layout_report,
        measure: runtime.shaper.borrow_mut().take_shape_stats(),
        scale: runtime.point_scale,
        paused,
        again,
        tree: runtime.latest.clone(),
        scene_delta,
        requirements: Arc::clone(&runtime.image_requirements),
        platform_views: Arc::clone(&runtime.platform_views),
        placements_changed,
        layers_changed,
        reloads,
        census,
        hub: runtime.hub.clone(),
        compositor_epoch,
        compositor_frame: false,
        animated_effects: runtime.animated_effects,
        gpu_views: runtime.gpu_views,
    };
    drop(entered);
    if let Some(schedule) = submission.schedule.as_mut() {
        schedule.ui_done_ns = mono_ns();
    }
    if let Some(generation) = prefetch_generation {
        if prefetch_wait && !leave && !placements_changed && submission.reloads.is_empty() {
            compositor_animation::defer_window(runtime, generation);
            return Ok(false);
        }
        if !leave
            && runtime.compositor_mode.publish_window(
                generation,
                runtime.compositor_epoch,
                submission,
            )
        {
            refresh_wake(runtime);
            return Ok(false);
        }
        compositor_animation::stop(runtime, None);
        if leave {
            return Ok(end_frame(runtime, false, true));
        }
        arm(runtime);
        return Ok(false);
    }
    submit_frame(runtime, submission, raster_admitted)
}

fn submit_frame(
    runtime: &mut UiHost,
    submission: FrameSubmission,
    raster_admitted: bool,
) -> Result<bool, BootError> {
    let again = submission.again;
    let compositor_epoch = submission.compositor_epoch;
    if !raster_admitted {
        skipped_frame("checkpoint_deferred");
        runtime.frames.keep_latest(submission);
        return Ok(end_frame(runtime, false, false));
    }
    let submission = if runtime.frames.pending.is_some() {
        runtime.frames.keep_latest(submission);
        if let Some(pending) = runtime.frames.pending.take() {
            *pending
        } else {
            return Err(BootError::Panicked);
        }
    } else {
        submission
    };
    match &mut runtime.graphics {
        RasterPort::Local(graphics) => {
            let completion = raster_frame(graphics, submission, false, || {
                if let Some(token) = runtime.scheduler.prepare_next() {
                    runtime.vsync.request_frame(token);
                }
            });
            finish_raster_frame(runtime, completion, false)
        }
        RasterPort::Remote(slot) => {
            let first_tree = runtime.queue_placement == "ui-boot" && submission.tree.is_some();
            post_raster_frame(slot, &runtime.inbox, &mut runtime.frames, submission)?;
            runtime.handoff_ready |= first_tree;
            Ok(end_frame(
                runtime,
                compositor_epoch.is_none()
                    && again
                    && runtime.frames.in_flight < FramePipeline::DEPTH,
                false,
            ))
        }
    }
}

fn post_raster_frame(
    slot: &Arc<RunnerSlot<RasterHost>>,
    inbox: &Arc<Inbox>,
    frames: &mut FramePipeline,
    submission: FrameSubmission,
) -> Result<(), BootError> {
    let slot = Arc::clone(slot);
    let inbox = Arc::clone(inbox);
    frames.submit();
    let posted = Arc::clone(slot.runner()).post(Box::new(move || {
        let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            slot.with(|graphics| raster_frame(graphics, submission, true, || {}))
                .unwrap_or(Err(BootError::Panicked))
        }))
        .unwrap_or(Err(BootError::Panicked));
        inbox.complete(Box::new(move |runtime| {
            finish_raster_frame(runtime, completion, true)
        }));
    }));
    if !posted {
        frames.complete();
        skipped_frame("raster_closed");
        return Err(BootError::Panicked);
    }
    Ok(())
}

fn note_admission_failure(
    runtime: &UiHost,
    schedule: Option<ScheduleTiming>,
    budget_ms: f64,
    reason: &'static str,
) {
    skipped_frame(reason);
    if let (Some(hub), Some(mut schedule)) = (&runtime.telemetry, schedule)
        && hub.collects_frames()
    {
        schedule.ui_done_ns = mono_ns();
        schedule.skip_reason = Some(reason);
        hub.push_frame(crate::diagnostics::FrameSample {
            number: next_frame_number(),
            at_ms: crate::timing::mono_ms(),
            budget_ms,
            schedule,
            ..Default::default()
        });
    }
}

pub(super) fn skipped_frame(status: &'static str) {
    tracing::event!(name: "frame_skipped", target: "impasto::frame", tracing::Level::DEBUG,
        trace_event = true, queue = "ui", status);
}

fn finish_raster_frame(
    runtime: &mut UiHost,
    completion: Result<RasterCompletion, BootError>,
    split: bool,
) -> Result<bool, BootError> {
    let waiting = split && runtime.frames.complete();
    let completion = completion?;
    let activated = split && compositor_animation::activate(runtime, &completion);
    let RasterCompletion {
        submission,
        output,
        surface,
        retry_at,
    } = completion;
    if !activated && output.gpu_pending {
        schedule_gpu_poll(runtime);
    }
    if let Some(timing) = output.timing {
        for listener in &mut runtime.listeners {
            listener.on_frame(&timing, &submission.phases);
        }
        if runtime.inspect_dir.is_none()
            && let Some(dir) = &runtime.inspect
            && let Err(error) =
                crate::timing::write_inspect(dir, &timing, submission.tree.as_deref())
        {
            tracing::warn!(target: "impasto::shell", %error, "inspect write failed");
        }
        if (submission.revision == runtime.revision || output.stop)
            && output.shown
            && let Some(tree) = submission.tree
        {
            write_inspect(
                runtime,
                &tree,
                &timing,
                &submission.phases,
                issue_list(submission.layout_report.as_deref().unwrap_or("[]")),
            );
        }
    }
    if output.stop {
        runtime.services.borrow().request_exit();
        return Ok(end_frame(runtime, false, true));
    }
    if submission.revision != runtime.revision
        || surface.is_some_and(|id| !runtime.inbox.admits_surface(id))
    {
        if waiting && !runtime.input.is_paused() {
            arm(runtime);
        }
        return Ok(false);
    }
    runtime.active_surface = surface.filter(|&id| runtime.inbox.admits_surface(id));
    runtime.present_retry_at = retry_at;
    if output.shown {
        if submission.interaction == runtime.interaction {
            runtime.interaction = None;
        }
        runtime.busy.arm();
        if let Some(surface) = surface {
            let views = submission.platform_views;
            let changed =
                runtime
                    .published_views
                    .as_ref()
                    .map_or(!views.is_empty(), |(id, prior)| {
                        (*id == surface && !Arc::ptr_eq(prior, &views))
                            || (*id != surface && !views.is_empty())
                    });
            if changed {
                runtime.services.borrow().platform_views(surface, &views);
                runtime.published_views = Some((surface, views));
            }
        }
    }
    if activated {
        return Ok(false);
    }
    if split
        && !runtime.input.is_paused()
        && let Some(pending) = runtime.frames.pending.take()
        && pending.revision == runtime.revision
        && let RasterPort::Remote(slot) = &runtime.graphics
    {
        post_raster_frame(slot, &runtime.inbox, &mut runtime.frames, *pending)?;
    }
    let again = (waiting || output.again) && !runtime.input.is_paused();
    if split {
        refresh_wake(runtime);
        if again {
            arm(runtime);
        }
        return Ok(false);
    }
    Ok(end_frame(runtime, again, false))
}

fn frame_schedule(
    runtime: &UiHost,
    time: FrameTime,
    frame_started: Instant,
    counters: bool,
    from_vsync: bool,
) -> Option<ScheduleTiming> {
    let inspect = runtime.inspect.is_some() || runtime.inspect_dir.is_some();
    (counters || inspect).then(|| {
        let ui_start_ns = mono_ns_at(frame_started);
        // FrameTime::mono_ms was stamped when the callback posted to the
        // inbox. Use that runner's elapsed time so custom epochs also work.
        let vsync_ns = from_vsync.then(|| {
            let queued_ms = (runtime.wake.runner.monotonic_ms() - time.mono_ms).max(0.0);
            ui_start_ns.saturating_sub(ms_to_ns(queued_ms))
        });
        ScheduleTiming {
            vsync_ns,
            ui_start_ns,
            topology: Some(runtime.queue_placement),
            in_flight_depth: u32::try_from(runtime.frames.in_flight).ok(),
            budget_ns: ms_to_ns(time.interval_ms),
            ..ScheduleTiming::default()
        }
    })
}

fn deliver_frame_input(runtime: &mut UiHost) -> (bool, f64) {
    let frame = runtime.input.begin_frame();
    if let Some(metrics) = frame.metrics {
        runtime.point_scale = metrics.device_pixel_ratio;
        if let Some(session) = runtime.session.as_mut() {
            session_call(|| session.metrics(&metrics));
        }
    }
    let gesture_started = Instant::now();
    let input_trace = tracing::debug_span!(target: "impasto::frame", "input").entered();
    if let Some(session) = runtime.session.as_mut() {
        for event in frame.events {
            if tracing::enabled!(target: "impasto::shell", tracing::Level::TRACE)
                && let FrameInput::Pointer(event) | FrameInput::PointerMoves { event, .. } = &event
            {
                tracing::trace!(
                    target: "impasto::shell",
                    kind = phase_name(event.phase),
                    x = event.x,
                    y = event.y,
                    "pointer"
                );
            }
            match event {
                FrameInput::Pointer(event) => {
                    session_call(|| session.pointer(event));
                }
                FrameInput::PointerMoves { event, history } => {
                    session_call(|| session.pointer_moves(event, history.as_slice()));
                }
                FrameInput::Session(event) => {
                    tracing::trace!(target: "impasto::shell", kind = input_kind(&event), "input");
                    session_call(|| session.input(event));
                }
                FrameInput::DismissIme => {
                    tracing::trace!(target: "impasto::shell", "ime dismissed");
                    session_call(|| session.ime_dismissed());
                }
            }
        }
    }
    let gesture_ms = gesture_started.elapsed().as_secs_f64() * 1000.0;
    drop(input_trace);
    (frame.paused, gesture_ms)
}

struct SessionFrame {
    again: bool,
    leave: bool,
    reloads: Vec<String>,
    census: Option<Arc<engine::semantics::ImageCensus>>,
    layers_changed: bool,
    scene_delta: Option<SceneDelta>,
    placements_changed: bool,
    phases: Option<FramePhases>,
    runtime_sample: Option<RuntimeSample>,
    layout_report: Option<String>,
    compositor_epoch: Option<u64>,
    prefetch_wait: bool,
}

fn run_session(
    runtime: &mut UiHost,
    time: FrameTime,
    counters: bool,
    paused: bool,
    prefetch_generation: Option<u64>,
    schedule: &mut Option<ScheduleTiming>,
) -> Option<SessionFrame> {
    let mut leave = false;
    let mut again;
    let mut reloads = Vec::new();
    let mut census = None;
    let mut layers_changed = false;
    let mut scene_delta = None;
    let mut placements_changed = false;
    let mut phases = None;
    let mut runtime_sample = None;
    let mut layout_report = None;
    let mut compositor_epoch = None;
    let mut prefetch_wait = false;
    let session = runtime.session.as_mut()?;
    session.set_counters(counters);
    let framed = {
        let _trace = tracing::debug_span!(target: "impasto::frame", "session").entered();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match prefetch_generation {
            Some(_) => session.prefetch_compositor_window(time),
            None => session.frame(time),
        }))
    };
    match framed {
        Ok(mut output) => {
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            if let Some(hub) = runtime.telemetry.as_ref() {
                if let Some(schedule) = schedule {
                    schedule.repainted_view_count = output.repaints.as_ref().map_or(0, |batch| {
                        u32::try_from(batch.total_repainted_views).unwrap_or(u32::MAX)
                    });
                }
                hub.note_repaints(output.repaints.take(), session.take_picked_view());
                if let Some(tree) = output.layers.as_ref().or(runtime.latest.as_ref()) {
                    hub.note_layers(runtime.frame_number, tree);
                }
                hub.refresh_inspector(!output.again, || session.inspector_snapshot());
            }
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            let reads_runtime = runtime
                .telemetry
                .as_deref()
                .is_some_and(crate::diagnostics::TelemetryHub::reads_runtime);
            #[cfg(not(any(debug_assertions, feature = "devtools-server")))]
            let reads_runtime = false;
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            let reads_resources = runtime
                .telemetry
                .as_deref()
                .is_some_and(crate::diagnostics::TelemetryHub::reads_resources);
            #[cfg(not(any(debug_assertions, feature = "devtools-server")))]
            let reads_resources = false;
            if (counters || reads_runtime || reads_resources)
                && runtime
                    .last_runtime_sample
                    .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(1))
            {
                runtime.last_runtime_sample = Some(Instant::now());
                let memory = (counters || reads_runtime)
                    .then(|| session.js_memory_detail())
                    .flatten();
                if counters {
                    runtime_sample = session.runtime_counters().map(|mut sample| {
                        sample.heap_bytes = memory
                            .and_then(|detail| usize::try_from(detail.memory_used_size).ok())
                            .or(sample.heap_bytes);
                        sample
                    });
                }
                #[cfg(any(debug_assertions, feature = "devtools-server"))]
                if let Some(hub) = &runtime.telemetry {
                    use crate::diagnostics::remote_runtime;
                    if reads_runtime && let Some(detail) = memory {
                        hub.note_runtime(serde_json::json!({"kind": "memory", "detail": remote_runtime::memory(detail)}));
                    }
                    if reads_resources && let Some(snapshot) = session.js_resources_snapshot() {
                        hub.note_resources(serde_json::json!({"kind": "async", "detail": remote_runtime::resources(snapshot)}));
                    }
                }
            }
            #[cfg(any(debug_assertions, feature = "devtools-server"))]
            if let Some(hub) = &runtime.telemetry {
                hub.note_js_samples(runtime.frame_number, session.take_js_stack_samples());
            }
            again = output.again;
            leave = output.exit;
            let faults = session.take_faults();
            let plan = fault_followup(runtime.diag_armed, &faults);
            runtime.diag_armed = plan.diag_armed;
            if plan.stop_again {
                again = false;
            }
            if let Some(hub) = runtime.hub.as_ref() {
                for fault in &faults {
                    hub.error(
                        fault.source,
                        &fault.message,
                        (!fault.stack.is_empty()).then_some(fault.stack.as_str()),
                        Some(runtime.frame_number),
                    );
                }
            }
            if let Some(tree) = output.layers.take() {
                if runtime.dump_list {
                    tracing::debug!(
                        target: "impasto::render",
                        "{}",
                        layer_tree_line(&tree)
                    );
                }
                scene_delta = output.scene_delta.take().filter(|delta| {
                    runtime
                        .latest
                        .as_ref()
                        .is_some_and(|old| old.epoch == delta.base_epoch)
                });
                if scene_delta.as_ref().is_none_or(|delta| {
                    !matches!(delta.changes, engine::scene::SceneChanges::LayerValues)
                }) {
                    runtime.image_requirements = tree.image_requirements().into();
                    runtime.animated_effects = tree.has_animated_effects();
                    runtime.gpu_views = tree.has_gpu_views();
                }
                runtime.latest = Some(tree);
                layers_changed = true;
            }
            if !paused
                && !leave
                && faults.is_empty()
                && let Some(tree) = runtime.latest.as_ref()
                && (again
                    || runtime.animated_effects
                    || runtime.gpu_views
                    || prefetch_generation.is_some())
            {
                use engine::scene::CompositorBlockReason;
                let has_compositor_work =
                    !tree.tracks.is_empty() || runtime.animated_effects || runtime.gpu_views;
                let eligible = session.compositor_eligibility(&tree.tracks).and({
                    if !has_compositor_work || runtime.compositor_enabled {
                        Ok(())
                    } else if matches!(runtime.graphics, RasterPort::Local(_)) {
                        Err(CompositorBlockReason::SharedTopology)
                    } else {
                        Err(CompositorBlockReason::Disabled)
                    }
                });
                match eligible {
                    Ok(()) if has_compositor_work => {
                        compositor_epoch = Some(runtime.compositor_epoch);
                    }
                    Ok(()) => {}
                    Err(reason) => {
                        prefetch_wait =
                            matches!(reason, CompositorBlockReason::ListWindowNotSettled { .. });
                        if let Some(schedule) = schedule.as_mut() {
                            schedule.compositor_block = Some(reason);
                        }
                        tracing::event!(name: "compositor_deferred", target: "impasto::compositor",
                            tracing::Level::DEBUG, trace_event = true, queue = "ui",
                            status = reason.name(), node = reason.node(),
                            active_tracks = tree.tracks.len());
                    }
                }
            }
            if plan.arm_extra {
                arm(runtime);
            }
            placements_changed = apply_session_updates(runtime, &mut output);
            reloads = std::mem::take(&mut output.reloads);
            census = output.images.take();
            phases = output.phases.take();
            layout_report = output.layout_report.take();
            if let Some(hub) = runtime.hub.as_ref()
                && let Some(report) = layout_report.as_deref()
            {
                hub.set_layout_report(report);
            }
        }
        Err(_) => {
            tracing::error!(
                target: "impasto::script",
                "session call panicked"
            );
            again = false;
        }
    }
    if leave {
        runtime.services.borrow().request_exit();
    }
    Some(SessionFrame {
        again,
        leave,
        reloads,
        census,
        layers_changed,
        scene_delta,
        placements_changed,
        phases,
        runtime_sample,
        layout_report,
        compositor_epoch,
        prefetch_wait,
    })
}

fn phase_name(phase: engine::input::Phase) -> &'static str {
    match phase {
        engine::input::Phase::Down => "down",
        engine::input::Phase::Move => "move",
        engine::input::Phase::Up => "up",
        engine::input::Phase::Cancel => "cancel",
        engine::input::Phase::Wheel => "wheel",
        engine::input::Phase::Zoom => "zoom",
    }
}

fn input_kind(event: &engine::session::InputEvent) -> &'static str {
    match event {
        engine::session::InputEvent::Key(_) | engine::session::InputEvent::Back => "key",
        engine::session::InputEvent::Text(_) | engine::session::InputEvent::Paste { .. } => "text",
        engine::session::InputEvent::Composing(_) | engine::session::InputEvent::DismissIme(_) => {
            "ime"
        }
        engine::session::InputEvent::Copy
        | engine::session::InputEvent::Cut
        | engine::session::InputEvent::SelectAll
        | engine::session::InputEvent::Edit(_)
        | engine::session::InputEvent::Replace { .. }
        | engine::session::InputEvent::ReplaceIme { .. } => "edit",
    }
}

fn apply_session_updates(runtime: &mut UiHost, output: &mut FrameOutput) -> bool {
    if let Some(text) = output.clipboard.take()
        && let Err(message) = runtime.registry.call(
            "Clipboard",
            "setString",
            &[engine::session::PluginValue::Str(text)],
        )
    {
        tracing::error!(target: "impasto::shell", "{message}");
    }
    if let Some(update) = output.ime.take() {
        crate::ime::publish_update(&update, &runtime.services);
    }
    if let Some(patch) = output.semantics.take() {
        let _trace = tracing::debug_span!(target: "impasto::frame", "accessibility").entered();
        let services = runtime.services.borrow();
        let projection = runtime
            .semantics
            .get_or_insert_with(|| crate::semantics::Projection::new(patch.root));
        if let Some(nodes) = projection.apply(&patch, services.semantics_include_platform_views()) {
            services.semantics(nodes);
        }
    }
    if let Some(placements) = output.platform_views.take() {
        runtime.platform_views = placements;
        return true;
    }
    false
}
