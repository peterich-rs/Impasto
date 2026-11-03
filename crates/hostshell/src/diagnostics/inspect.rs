//! One-shot `frame.json` for `IMPASTO_INSPECT`. Written on the render thread
//! after a successful present, next to the dump PNG when both are set.

use std::path::Path;

use protocol::{LayerId, LayerKind, LayerTree};

use crate::timing::FrameTiming;

/// Writes `<dir>/frame.json`. Creates the directory. The caller exits the
/// process after the first successful write, the same way a dump does.
pub fn write_frame(
    dir: &Path,
    tree: &LayerTree,
    timing: &FrameTiming,
    phases: Option<&session::FramePhases>,
    issues_json: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut out = String::new();
    out.push('{');
    push_key(&mut out, "frame");
    push_u64(&mut out, timing.number);
    out.push(',');
    push_key(&mut out, "timing");
    timing_json(&mut out, timing);
    out.push(',');
    push_key(&mut out, "phases");
    match phases {
        Some(phases) => phases_json(&mut out, phases),
        None => out.push_str("null"),
    }
    out.push(',');
    push_key(&mut out, "layers");
    out.push('[');
    let mut first = true;
    write_layer(tree, tree.root, &mut out, &mut first);
    out.push(']');
    out.push(',');
    push_key(&mut out, "issues");
    if issues_json.starts_with('[') || issues_json.starts_with('{') {
        out.push_str(issues_json);
    } else {
        out.push_str("[]");
    }
    out.push('}');
    out.push('\n');
    std::fs::write(dir.join("frame.json"), out)
}

fn write_layer(tree: &LayerTree, id: LayerId, out: &mut String, first: &mut bool) {
    let Some(layer) = tree.layers.get(id.0 as usize) else {
        return;
    };
    if !*first {
        out.push(',');
    }
    *first = false;
    out.push('{');
    push_key(out, "id");
    push_u64(out, u64::from(id.0));
    out.push(',');
    push_key(out, "kind");
    out.push('"');
    out.push_str(kind_name(&layer.kind));
    out.push('"');
    if let LayerKind::Picture { picture } = layer.kind {
        out.push(',');
        push_key(out, "picture");
        push_u64(out, picture.0);
        out.push(',');
        push_key(out, "commands");
        let count = tree
            .picture(picture)
            .map(|picture| picture.commands.len())
            .unwrap_or(0);
        push_u64(out, count as u64);
        if let Some(picture) = tree.picture(picture) {
            out.push(',');
            push_key(out, "origin");
            out.push('[');
            push_f32(out, picture.origin.0);
            out.push(',');
            push_f32(out, picture.origin.1);
            out.push(']');
        }
    }
    out.push(',');
    push_key(out, "children");
    out.push('[');
    let mut child_first = true;
    let mut child = layer.first_child;
    while let Some(id) = child {
        write_layer(tree, id, out, &mut child_first);
        child = tree.layers.get(id.0 as usize).and_then(|layer| layer.next);
    }
    out.push_str("]}");
}

fn kind_name(kind: &LayerKind) -> &'static str {
    match kind {
        LayerKind::Offset { .. } => "offset",
        LayerKind::Transform { .. } => "transform",
        LayerKind::Clip { .. } => "clip",
        LayerKind::Opacity { .. } => "opacity",
        LayerKind::Blur { .. } => "blur",
        LayerKind::BackdropBlur { .. } => "backdropBlur",
        LayerKind::Blend { .. } => "blend",
        LayerKind::Picture { .. } => "picture",
    }
}

fn timing_json(out: &mut String, timing: &FrameTiming) {
    out.push('{');
    field_f64(out, "sessionMs", timing.session_ms, true);
    field_f64(out, "gestureMs", timing.gesture_ms, false);
    field_f64(out, "scriptMs", timing.script_ms, false);
    field_f64(out, "layoutMs", timing.layout_ms, false);
    field_f64(out, "layoutCbMs", timing.layout_cb_ms, false);
    field_f64(out, "syncMs", timing.sync_ms, false);
    field_f64(out, "recordMs", timing.record_ms, false);
    field_f64(out, "semanticsMs", timing.semantics_ms, false);
    field_f64(out, "submitMs", timing.submit_ms, false);
    field_f64(out, "presentMs", timing.present_ms, false);
    field_u64(out, "layers", u64::from(timing.layer_count), false);
    field_u64(out, "paragraphs", u64::from(timing.paragraphs), false);
    field_u64(out, "pictures", u64::from(timing.pictures), false);
    field_u64(out, "textures", u64::from(timing.textures), false);
    out.push('}');
}

fn phases_json(out: &mut String, phases: &session::FramePhases) {
    out.push('{');
    field_f64(out, "motionMs", phases.script.motion_ms, true);
    field_f64(out, "dispatchMs", phases.script.dispatch_ms, false);
    field_f64(out, "timerMs", phases.script.timer_ms, false);
    field_f64(out, "rafMs", phases.script.raf_ms, false);
    field_f64(out, "callbackMs", phases.script.callback_ms, false);
    field_f64(out, "flushMs", phases.script.flush_ms, false);
    field_f64(out, "layoutCbMs", phases.layout_cb_ms, false);
    field_f64(out, "syncMs", phases.sync_ms, false);
    field_f64(out, "layworkMs", phases.trace.laywork_ms, false);
    field_u64(out, "passes", u64::from(phases.trace.layout_passes), false);
    field_u64(out, "reuse", u64::from(phases.trace.text_reused), false);
    field_u64(out, "shaped", u64::from(phases.trace.text_shaped), false);
    field_u64(out, "pointers", u64::from(phases.trace.pointers), false);
    field_u64(out, "keys", u64::from(phases.trace.keys), false);
    out.push('}');
}

fn field_f64(out: &mut String, key: &str, value: f64, first: bool) {
    if !first {
        out.push(',');
    }
    push_key(out, key);
    push_f64(out, value);
}

fn field_u64(out: &mut String, key: &str, value: u64, first: bool) {
    if !first {
        out.push(',');
    }
    push_key(out, key);
    push_u64(out, value);
}

fn push_key(out: &mut String, key: &str) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
}

fn push_u64(out: &mut String, value: u64) {
    out.push_str(&value.to_string());
}

fn push_f32(out: &mut String, value: f32) {
    push_f64(out, f64::from(value));
}

fn push_f64(out: &mut String, value: f64) {
    if value.is_finite() {
        out.push_str(&format!("{value:.3}"));
    } else {
        out.push('0');
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use protocol::{Picture, PictureId};

    use super::*;

    fn picture(id: u64, commands: usize) -> Arc<Picture> {
        Arc::new(Picture {
            id: PictureId(id),
            generation: 1,
            origin: (1.5, 2.5),
            commands: Arc::from(vec![
                protocol::Command::Fill {
                    color: [0, 0, 0, 0],
                };
                commands
            ]),
        })
    }

    fn timing(number: u64) -> FrameTiming {
        FrameTiming {
            number,
            vsync_ms: 0.0,
            target_ms: 0.0,
            session_ms: 1.25,
            gesture_ms: 0.0,
            script_ms: 0.5,
            layout_ms: 0.4,
            layout_cb_ms: 0.0,
            sync_ms: 0.0,
            record_ms: 0.0,
            semantics_ms: 0.0,
            submit_ms: 0.2,
            present_ms: 0.1,
            shapes_built: 0,
            shapes_measure_hit: 0,
            shapes_measure_miss: 0,
            shapes_paint_hit: 0,
            shapes_paint_miss: 0,
            picture_rebuilt: 0,
            picture_reused: 0,
            layer_count: 3,
            paragraphs: 1,
            pictures: 1,
            textures: 0,
        }
    }

    #[test]
    fn a_nested_tree_writes_layers_and_timing() {
        let tree = LayerTree {
            layers: vec![
                protocol::Layer {
                    kind: LayerKind::Offset { x: 0.0, y: 0.0 },
                    parent: None,
                    first_child: Some(LayerId(1)),
                    next: None,
                },
                protocol::Layer {
                    kind: LayerKind::Clip {
                        x: 0.0,
                        y: 0.0,
                        w: 10.0,
                        h: 10.0,
                        corner_radius: 0.0,
                    },
                    parent: Some(LayerId(0)),
                    first_child: Some(LayerId(2)),
                    next: None,
                },
                protocol::Layer {
                    kind: LayerKind::Picture {
                        picture: PictureId(7),
                    },
                    parent: Some(LayerId(1)),
                    first_child: None,
                    next: None,
                },
            ],
            pictures: vec![picture(7, 3)],
            root: LayerId(0),
            epoch: 1,
        };
        let dir = std::env::temp_dir().join(format!("impasto-inspect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let phases = session::FramePhases {
            script_ms: 0.5,
            layout_ms: 0.4,
            ..session::FramePhases::default()
        };
        write_frame(&dir, &tree, &timing(4), Some(&phases), "[]").unwrap();
        let text = std::fs::read_to_string(dir.join("frame.json")).unwrap();
        assert!(text.contains("\"frame\":4"), "{text}");
        assert!(text.contains("\"kind\":\"offset\""), "{text}");
        assert!(text.contains("\"kind\":\"clip\""), "{text}");
        assert!(text.contains("\"kind\":\"picture\""), "{text}");
        assert!(text.contains("\"commands\":3"), "{text}");
        assert!(text.contains("\"issues\":[]"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
