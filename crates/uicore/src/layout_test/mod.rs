//! Layout, hit, and record tests for [`crate::Tree`].

use crate::tree::Affine;
use crate::{
    AxisSize, Button, DragAxis, Edges, Gesture, MonoShaper, Phase, PointerEvent, PointerId,
    RecognizerSet, Shaper, SubtreeSpace, TrackSize, Tree,
};

mod flex;
mod grid;
mod picture;
mod pointer;
mod scroll;
mod tree;

pub(super) fn event(phase: Phase, x: f32, y: f32) -> PointerEvent {
    PointerEvent::new(phase, x, y, 0.0)
}

pub(super) fn button_event(phase: Phase, x: f32, y: f32, button: Button) -> PointerEvent {
    let mut event = PointerEvent::new(phase, x, y, 0.0);
    event.button = button;
    event.buttons = crate::Buttons::from_button(button);
    event
}

pub(super) fn timed(phase: Phase, x: f32, y: f32, time_ms: f64) -> PointerEvent {
    PointerEvent::new(phase, x, y, time_ms)
}

pub(super) fn texts(tree: &mut Tree) -> Result<Vec<String>, crate::Error> {
    Ok(tree
        .record()?
        .commands
        .iter()
        .filter_map(|command| match command {
            protocol::Command::Text { utf8, .. } => Some(utf8.to_string()),
            _ => None,
        })
        .collect())
}

pub(super) fn picture_generation(tree: &protocol::LayerTree, id: crate::NodeId) -> u64 {
    tree.picture(crate::layer::child_picture_id(id))
        .map(|picture| picture.generation)
        .expect("picture")
}

pub(super) fn reachable(tree: &protocol::LayerTree) -> usize {
    fn walk(layers: &[protocol::Layer], id: protocol::LayerId, seen: &mut [bool]) {
        let index = id.0 as usize;
        if index >= layers.len() || seen[index] {
            return;
        }
        seen[index] = true;
        if let Some(child) = layers[index].first_child {
            walk(layers, child, seen);
        }
        if let Some(next) = layers[index].next {
            walk(layers, next, seen);
        }
    }
    if tree.layers.is_empty() {
        return 0;
    }
    let mut seen = vec![false; tree.layers.len()];
    walk(&tree.layers, tree.root, &mut seen);
    seen.into_iter().filter(|hit| *hit).count()
}

pub(super) fn agree(tree: &mut Tree) -> Result<(), crate::Error> {
    let list = tree.record()?;
    agree_list(tree, &list)
}

pub(super) fn record_writes(tree: &mut Tree) -> Result<usize, crate::Error> {
    let _ = tree.record()?;
    Ok(tree.picture_writes())
}

pub(super) fn agree_list(tree: &Tree, list: &protocol::DrawList) -> Result<(), crate::Error> {
    let layers = tree.layer_tree();
    let flat = scene_marks_in_commands(&list.commands, Affine::identity(), &[]);
    let layered = scene_marks_in_layers(&layers);
    assert_eq!(
        flat.len(),
        layered.len(),
        "flat {flat:?}\nlayers {layered:?}"
    );
    for (left, right) in flat.iter().zip(layered.iter()) {
        assert_eq!(left.label, right.label, "{flat:?} vs {layered:?}");
        assert!(
            (left.x - right.x).abs() < 0.05 && (left.y - right.y).abs() < 0.05,
            "{left:?} vs {right:?}"
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct Mark {
    label: String,
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
pub(super) struct SceneClip {
    ctm: Affine,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub(super) fn inside_clips(point: (f32, f32), clips: &[SceneClip]) -> bool {
    clips.iter().all(|clip| {
        let (x0, y0) = clip.ctm.apply(clip.x, clip.y);
        let (x1, y1) = clip.ctm.apply(clip.x + clip.w, clip.y + clip.h);
        let (min_x, max_x) = (x0.min(x1), x0.max(x1));
        let (min_y, max_y) = (y0.min(y1), y0.max(y1));
        point.0 >= min_x - 0.01
            && point.0 <= max_x + 0.01
            && point.1 >= min_y - 0.01
            && point.1 <= max_y + 0.01
    })
}

pub(super) fn push_mark(
    marks: &mut Vec<Mark>,
    label: String,
    x: f32,
    y: f32,
    ctm: Affine,
    clips: &[SceneClip],
) {
    let point = ctm.apply(x, y);
    if inside_clips(point, clips) {
        marks.push(Mark {
            label,
            x: point.0,
            y: point.1,
        });
    }
}

pub(super) fn scene_marks_in_commands(
    commands: &[protocol::Command],
    ctm: Affine,
    clips: &[SceneClip],
) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut stack = vec![(ctm, clips.to_vec())];
    for command in commands {
        let (ctm, clips) = stack.last().expect("frame").clone();
        match command {
            protocol::Command::Text { utf8, x, y, .. } => {
                push_mark(&mut marks, utf8.to_string(), *x, *y, ctm, &clips);
            }
            protocol::Command::Image { source, x, y, .. } => {
                push_mark(&mut marks, source.to_string(), *x, *y, ctm, &clips);
            }
            protocol::Command::Rect { x, y, color, .. } => {
                push_mark(&mut marks, format!("rect:{color:?}"), *x, *y, ctm, &clips);
            }
            protocol::Command::PushClip { x, y, w, h, .. } => {
                let mut clips = clips;
                clips.push(SceneClip {
                    ctm,
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                });
                stack.push((ctm, clips));
            }
            protocol::Command::PushTransform {
                translate_x,
                translate_y,
                rotate_rad,
                scale_x,
                scale_y,
            } => {
                stack.push((
                    Affine::paint(*translate_x, *translate_y, *rotate_rad, *scale_x, *scale_y)
                        .around(ctm),
                    clips,
                ));
            }
            protocol::Command::PushOpacity { .. }
            | protocol::Command::PushBlur { .. }
            | protocol::Command::PushBackdropBlur { .. }
            | protocol::Command::PushBlend { .. } => {
                stack.push((ctm, clips));
            }
            protocol::Command::Pop => {
                stack.pop();
            }
            protocol::Command::Fill { .. } => {}
        }
    }
    marks
}

pub(super) fn scene_marks_in_layers(tree: &protocol::LayerTree) -> Vec<Mark> {
    fn walk(
        tree: &protocol::LayerTree,
        id: protocol::LayerId,
        ctm: Affine,
        clips: &[SceneClip],
        marks: &mut Vec<Mark>,
    ) {
        let mut current = Some(id);
        while let Some(layer_id) = current {
            let Some(layer) = tree.layers.get(layer_id.0 as usize) else {
                return;
            };
            let next = layer.next;
            let child = layer.first_child;
            match layer.kind {
                protocol::LayerKind::Offset { x, y } => {
                    if let Some(child) = child {
                        walk(
                            tree,
                            child,
                            Affine::translate(x, y).around(ctm),
                            clips,
                            marks,
                        );
                    }
                }
                protocol::LayerKind::Transform {
                    translate_x,
                    translate_y,
                    rotate_rad,
                    scale_x,
                    scale_y,
                } => {
                    if let Some(child) = child {
                        walk(
                            tree,
                            child,
                            Affine::paint(translate_x, translate_y, rotate_rad, scale_x, scale_y)
                                .around(ctm),
                            clips,
                            marks,
                        );
                    }
                }
                protocol::LayerKind::Clip { x, y, w, h, .. } => {
                    if let Some(child) = child {
                        let mut clips = clips.to_vec();
                        clips.push(SceneClip { ctm, x, y, w, h });
                        walk(tree, child, ctm, &clips, marks);
                    }
                }
                protocol::LayerKind::Opacity { .. }
                | protocol::LayerKind::Blur { .. }
                | protocol::LayerKind::BackdropBlur { .. }
                | protocol::LayerKind::Blend { .. } => {
                    if let Some(child) = child {
                        walk(tree, child, ctm, clips, marks);
                    }
                }
                protocol::LayerKind::Picture { picture } => {
                    if let Some(picture) = tree.picture(picture) {
                        let local =
                            Affine::translate(picture.origin.0, picture.origin.1).around(ctm);
                        marks.extend(scene_marks_in_commands(
                            picture.commands.as_ref(),
                            local,
                            clips,
                        ));
                    }
                }
            }
            current = next;
        }
    }
    let mut marks = Vec::new();
    if !tree.layers.is_empty() {
        walk(tree, tree.root, Affine::identity(), &[], &mut marks);
    }
    marks
}
