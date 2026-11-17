//! Per-frame durations. Recording them does not change the frame.

#[derive(Clone, Copy, Debug)]
pub struct FrameTiming {
    pub number: u64,
    pub vsync_ms: f64,
    pub target_ms: f64,
    pub session_ms: f64,
    pub gesture_ms: f64,
    pub script_ms: f64,
    pub layout_ms: f64,
    pub layout_cb_ms: f64,
    pub sync_ms: f64,
    pub record_ms: f64,
    pub semantics_ms: f64,
    pub submit_ms: f64,
    pub present_ms: f64,
    pub shapes_built: u32,
    pub shapes_measure_hit: u32,
    pub shapes_measure_miss: u32,
    pub shapes_paint_hit: u32,
    pub shapes_paint_miss: u32,
    pub shapes_bridged: u32,
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layer_count: u32,
    /// Vsync interval used for [`FrameTiming::over_budget`].
    pub interval_ms: f64,
    /// Live engine cache entries after the submit: paragraphs, pictures,
    /// textures.
    pub paragraphs: u32,
    pub pictures: u32,
    pub textures: u32,
}

impl FrameTiming {
    pub fn over_budget(self) -> bool {
        self.interval_ms > 0.0
            && self.session_ms + self.submit_ms + self.present_ms > self.interval_ms
    }
}

/// Receives one finished frame. It does not start clocks.
pub trait FrameListener {
    fn on_frame(&mut self, timing: &FrameTiming, phases: &session::FramePhases);
}

/// Rising and falling edge of a boolean that stays true across frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Streak {
    active: bool,
    frames: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreakStep {
    Idle,
    Enter,
    Stay,
    Leave { frames: u32 },
}

impl Streak {
    fn observe(&mut self, hot: bool) -> StreakStep {
        if hot {
            self.frames = self.frames.saturating_add(1);
            if self.active {
                StreakStep::Stay
            } else {
                self.active = true;
                StreakStep::Enter
            }
        } else if self.active {
            let frames = self.frames;
            self.active = false;
            self.frames = 0;
            StreakStep::Leave { frames }
        } else {
            StreakStep::Idle
        }
    }
}

/// Logs the edge when a frame crosses the vsync budget, and when it returns.
#[derive(Default)]
pub struct BudgetEdge {
    over: Streak,
}

impl FrameListener for BudgetEdge {
    fn on_frame(&mut self, timing: &FrameTiming, phases: &session::FramePhases) {
        match self.over.observe(timing.over_budget()) {
            StreakStep::Enter => log_over_budget(timing, phases),
            StreakStep::Leave { frames } => {
                tracing::info!(
                    target: "impasto::render",
                    frames_over = frames,
                    "frame back under budget"
                );
            }
            StreakStep::Stay | StreakStep::Idle => {}
        }
    }
}

fn log_over_budget(timing: &FrameTiming, phases: &session::FramePhases) {
    let cause = phases.trace.layout_cause;
    let passes = phases.trace.layout_passes;
    let shaped = phases.trace.text_shaped;
    let scrolls = phases.trace.scrolls;
    let rebuilt = timing.picture_rebuilt;
    if script_dominates(timing) {
        let script = &phases.script;
        tracing::warn!(
            target: "impasto::render",
            session_ms = timing.session_ms,
            gesture_ms = timing.gesture_ms,
            script_ms = timing.script_ms,
            layout_ms = timing.layout_ms,
            record_ms = timing.record_ms,
            submit_ms = timing.submit_ms,
            present_ms = timing.present_ms,
            budget_ms = timing.interval_ms,
            layout_cause = cause,
            layout_passes = passes,
            text_shaped = shaped,
            picture_rebuilt = rebuilt,
            scrolls,
            shapes_built = timing.shapes_built,
            shapes_bridged = timing.shapes_bridged,
            measure_hit = timing.shapes_measure_hit,
            measure_miss = timing.shapes_measure_miss,
            paint_hit = timing.shapes_paint_hit,
            paint_miss = timing.shapes_paint_miss,
            motion_ms = script.motion_ms,
            dispatch_ms = script.dispatch_ms,
            timer_ms = script.timer_ms,
            raf_ms = script.raf_ms,
            callback_ms = script.callback_ms,
            flush_ms = script.flush_ms,
            "frame over budget"
        );
    } else {
        tracing::warn!(
            target: "impasto::render",
            session_ms = timing.session_ms,
            gesture_ms = timing.gesture_ms,
            script_ms = timing.script_ms,
            layout_ms = timing.layout_ms,
            record_ms = timing.record_ms,
            submit_ms = timing.submit_ms,
            present_ms = timing.present_ms,
            budget_ms = timing.interval_ms,
            layout_cause = cause,
            layout_passes = passes,
            text_shaped = shaped,
            picture_rebuilt = rebuilt,
            scrolls,
            shapes_built = timing.shapes_built,
            shapes_bridged = timing.shapes_bridged,
            measure_hit = timing.shapes_measure_hit,
            measure_miss = timing.shapes_measure_miss,
            paint_hit = timing.shapes_paint_hit,
            paint_miss = timing.shapes_paint_miss,
            "frame over budget"
        );
    }
}

fn script_dominates(timing: &FrameTiming) -> bool {
    let script = timing.script_ms;
    script > timing.gesture_ms
        && script > timing.layout_ms
        && script > timing.record_ms
        && script > timing.submit_ms
        && script > timing.present_ms
}

/// Edges for work that can stay under the vsync budget and still be wasteful.
#[derive(Default)]
pub struct WorkEdge {
    shaping: Streak,
    pictures: Streak,
}

impl FrameListener for WorkEdge {
    fn on_frame(&mut self, timing: &FrameTiming, phases: &session::FramePhases) {
        match self.shaping.observe(phases.trace.text_shaped > 0) {
            StreakStep::Enter => {
                tracing::info!(
                    target: "impasto::render::text",
                    text_shaped = phases.trace.text_shaped,
                    text_reused = phases.trace.text_reused,
                    layout_cause = phases.trace.layout_cause,
                    layout_passes = phases.trace.layout_passes,
                    "text shaping"
                );
            }
            StreakStep::Leave { frames } => {
                tracing::info!(target: "impasto::render::text", frames, "text shaping stopped");
            }
            StreakStep::Stay | StreakStep::Idle => {}
        }
        match self.pictures.observe(timing.picture_rebuilt > 0) {
            StreakStep::Enter => {
                tracing::info!(
                    target: "impasto::render::picture",
                    picture_rebuilt = timing.picture_rebuilt,
                    picture_reused = timing.picture_reused,
                    layer_count = timing.layer_count,
                    "picture rebuild"
                );
            }
            StreakStep::Leave { frames } => {
                tracing::info!(
                    target: "impasto::render::picture",
                    frames,
                    "picture rebuild stopped"
                );
            }
            StreakStep::Stay | StreakStep::Idle => {}
        }
    }
}

/// Copies the frame into the devtools frames ring.
pub struct DevtoolFrames {
    devtools: std::sync::Arc<crate::diagnostics::DiagnosticsHub>,
}

impl DevtoolFrames {
    pub fn new(devtools: std::sync::Arc<crate::diagnostics::DiagnosticsHub>) -> Self {
        Self { devtools }
    }
}

impl FrameListener for DevtoolFrames {
    fn on_frame(&mut self, timing: &FrameTiming, phases: &session::FramePhases) {
        self.devtools.push_frame(crate::diagnostics::FrameSample {
            seq: 0,
            number: timing.number,
            at_ms: crate::net::mono_ms(),
            session_ms: timing.session_ms,
            gesture_ms: timing.gesture_ms,
            script_ms: timing.script_ms,
            layout_ms: timing.layout_ms,
            record_ms: timing.record_ms,
            submit_ms: timing.submit_ms,
            present_ms: timing.present_ms,
            layers: timing.layer_count,
            paragraphs: timing.paragraphs,
            pictures: timing.pictures,
            textures: timing.textures,
            shapes_built: timing.shapes_built,
            shapes_measure_hit: timing.shapes_measure_hit,
            shapes_measure_miss: timing.shapes_measure_miss,
            shapes_paint_hit: timing.shapes_paint_hit,
            shapes_paint_miss: timing.shapes_paint_miss,
            shapes_bridged: timing.shapes_bridged,
            picture_rebuilt: timing.picture_rebuilt,
            picture_reused: timing.picture_reused,
            layout_cause: phases.trace.layout_cause,
            gestures: phases.trace.gestures,
            over_budget: timing.over_budget(),
            phases: crate::diagnostics::PhaseSample {
                motion_ms: phases.script.motion_ms,
                dispatch_ms: phases.script.dispatch_ms,
                timer_ms: phases.script.timer_ms,
                raf_ms: phases.script.raf_ms,
                callback_ms: phases.script.callback_ms,
                flush_ms: phases.script.flush_ms,
                layout_cb_ms: phases.layout_cb_ms,
                sync_ms: phases.sync_ms,
                semantics_ms: phases.semantics_ms,
                laywork_ms: phases.trace.laywork_ms,
                layout_passes: phases.trace.layout_passes,
                text_reused: phases.trace.text_reused,
                text_shaped: phases.trace.text_shaped,
                pointers: phases.trace.pointers,
                keys: phases.trace.keys,
                edits: phases.trace.edits,
                ime: phases.trace.ime,
                scrolls: phases.trace.scrolls,
                focus: phases.trace.focus,
                blur: phases.trace.blur,
            },
        });
    }
}

/// Writes `frame.json` for `IMPASTO_INSPECT`. Uses the timing already recorded
/// for the frame. Does not start another clock.
pub fn write_inspect(
    dir: &std::path::Path,
    timing: &FrameTiming,
    tree: Option<&protocol::LayerTree>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut body = String::new();
    body.push_str("{\n");
    field(&mut body, "number", &timing.number.to_string(), true);
    field(&mut body, "vsync_ms", &num(timing.vsync_ms), true);
    field(&mut body, "target_ms", &num(timing.target_ms), true);
    field(&mut body, "gesture_ms", &num(timing.gesture_ms), true);
    field(&mut body, "script_ms", &num(timing.script_ms), true);
    field(&mut body, "layout_ms", &num(timing.layout_ms), true);
    field(&mut body, "layout_cb_ms", &num(timing.layout_cb_ms), true);
    field(&mut body, "sync_ms", &num(timing.sync_ms), true);
    field(&mut body, "record_ms", &num(timing.record_ms), true);
    field(&mut body, "semantics_ms", &num(timing.semantics_ms), true);
    field(&mut body, "session_ms", &num(timing.session_ms), true);
    field(&mut body, "submit_ms", &num(timing.submit_ms), true);
    field(&mut body, "present_ms", &num(timing.present_ms), true);
    field(
        &mut body,
        "paragraphs",
        &timing.paragraphs.to_string(),
        true,
    );
    field(&mut body, "pictures", &timing.pictures.to_string(), true);
    field(&mut body, "textures", &timing.textures.to_string(), true);
    field(
        &mut body,
        "picture_rebuilt",
        &timing.picture_rebuilt.to_string(),
        true,
    );
    field(
        &mut body,
        "picture_reused",
        &timing.picture_reused.to_string(),
        true,
    );
    body.push_str("  \"layers\": [");
    if let Some(tree) = tree {
        for (index, layer) in tree.layers.iter().enumerate() {
            if index > 0 {
                body.push(',');
            }
            body.push_str("\n    {");
            body.push_str(&format!("\"index\":{index}"));
            body.push_str(&format!(",\"kind\":\"{}\"", layer_kind(&layer.kind)));
            if let protocol::LayerKind::Picture { picture } = layer.kind {
                let commands = tree
                    .picture(picture)
                    .map(|picture| picture.commands.len())
                    .unwrap_or(0);
                body.push_str(&format!(",\"picture\":{}", picture.0));
                body.push_str(&format!(",\"commands\":{commands}"));
            }
            body.push('}');
        }
        if !tree.layers.is_empty() {
            body.push('\n');
            body.push_str("  ");
        }
    }
    body.push_str("]\n}\n");
    std::fs::write(dir.join("frame.json"), body)
}

fn field(body: &mut String, name: &str, value: &str, comma: bool) {
    body.push_str("  \"");
    body.push_str(name);
    body.push_str("\": ");
    body.push_str(value);
    if comma {
        body.push(',');
    }
    body.push('\n');
}

fn num(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.3}")
    } else {
        "0".to_owned()
    }
}

fn layer_kind(kind: &protocol::LayerKind) -> &'static str {
    match kind {
        protocol::LayerKind::Offset { .. } => "offset",
        protocol::LayerKind::Transform { .. } => "transform",
        protocol::LayerKind::Clip { .. } => "clip",
        protocol::LayerKind::Opacity { .. } => "opacity",
        protocol::LayerKind::Blur { .. } => "blur",
        protocol::LayerKind::BackdropBlur { .. } => "backdrop",
        protocol::LayerKind::Blend { .. } => "blend",
        protocol::LayerKind::Picture { .. } => "picture",
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameTiming, write_inspect};

    #[test]
    fn inspect_writes_the_timing_fields_already_recorded() {
        let dir = std::env::temp_dir().join(format!("impasto-inspect-{}", std::process::id()));
        let timing = FrameTiming {
            number: 4,
            layout_ms: 1.5,
            submit_ms: 0.25,
            present_ms: 0.5,
            pictures: 2,
            textures: 1,
            paragraphs: 3,
            ..FrameTiming {
                number: 0,
                vsync_ms: 0.0,
                target_ms: 0.0,
                session_ms: 0.0,
                gesture_ms: 0.0,
                script_ms: 0.0,
                layout_ms: 0.0,
                layout_cb_ms: 0.0,
                sync_ms: 0.0,
                record_ms: 0.0,
                semantics_ms: 0.0,
                submit_ms: 0.0,
                present_ms: 0.0,
                shapes_built: 0,
                shapes_measure_hit: 0,
                shapes_measure_miss: 0,
                shapes_paint_hit: 0,
                shapes_paint_miss: 0,
                shapes_bridged: 0,
                picture_rebuilt: 0,
                picture_reused: 0,
                layer_count: 0,
                interval_ms: 0.0,
                paragraphs: 0,
                pictures: 0,
                textures: 0,
            }
        };
        write_inspect(&dir, &timing, None).expect("write");
        let text = std::fs::read_to_string(dir.join("frame.json")).expect("read");
        assert!(text.contains("\"number\": 4"));
        assert!(text.contains("\"layout_ms\": 1.500"));
        assert!(text.contains("\"submit_ms\": 0.250"));
        assert!(text.contains("\"present_ms\": 0.500"));
        assert!(text.contains("\"pictures\": 2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn streak_fires_on_the_edges_only() {
        let mut streak = super::Streak::default();
        assert_eq!(streak.observe(false), super::StreakStep::Idle);
        assert_eq!(streak.observe(true), super::StreakStep::Enter);
        assert_eq!(streak.observe(true), super::StreakStep::Stay);
        assert_eq!(
            streak.observe(false),
            super::StreakStep::Leave { frames: 2 }
        );
        assert_eq!(streak.observe(false), super::StreakStep::Idle);
    }
}
