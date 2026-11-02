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
    pub picture_rebuilt: u32,
    pub picture_reused: u32,
    pub layer_count: u32,
    /// Live engine cache entries after the submit: paragraphs, pictures,
    /// textures.
    pub paragraphs: u32,
    pub pictures: u32,
    pub textures: u32,
}

impl FrameTiming {
    pub fn over_budget(self, interval_ms: f64) -> bool {
        interval_ms > 0.0 && self.session_ms + self.submit_ms + self.present_ms > interval_ms
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
                picture_rebuilt: 0,
                picture_reused: 0,
                layer_count: 0,
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
}
