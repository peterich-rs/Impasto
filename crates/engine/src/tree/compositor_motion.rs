//! Picks motions whose source state can sleep until a compositor handoff.

use std::sync::Arc;

use crate::motion::{AnimProp, MotionCurve};
use crate::scene::{
    Command, CompositorProp, CompositorSnapshot, CompositorTrack, CompositorTransform, LayerId,
    LayerKind, ScrollBinding, ScrollCurve, ScrollSource, ScrollTargetProp,
};

use super::node::NodeFlags;
use super::picture_id::child_picture_id;
use super::scroll_rules::BindingSource;
use super::{ActiveMotion, NodeId, Tree};

/// Links a motion's node to the actual outer/content layers emitted by recording.
#[derive(Default)]
pub(super) struct MotionLayers {
    pub(super) transform: Option<(LayerId, CompositorTransform)>,
    pub(super) opacity: Option<LayerId>,
    pub(super) scroll: Option<LayerId>,
    pub(super) linked_transform: bool,
    pub(super) transition_opacity: bool,
    pub(super) blur: Option<LayerId>,
    pub(super) backdrop: Option<LayerId>,
    pub(super) fill: Option<LayerId>,
    pub(super) linked_effect: bool,
}

/// Geometry exported by a settled list driver after verifying its mounted rows.
#[derive(Clone, Copy)]
pub(super) struct ListCompositorWindow {
    pub bounds: [f32; 2],
    pub prefetch_edges: [Option<f32>; 2],
    pub horizontal_thumb: Option<ListCompositorLink>,
    pub vertical_thumb: Option<ListCompositorLink>,
    pub sticky: Option<ListCompositorLink>,
}

#[derive(Clone, Copy)]
pub(super) struct ListCompositorLink {
    pub node: NodeId,
    pub factor: f32,
    pub offset: f32,
    pub maximum: Option<f32>,
}

impl Tree {
    pub(super) fn begin_compositor_record(&mut self) {
        self.motion_layers.clear();
        for (&node, rules) in &self.scroll_bindings {
            let layers = self.motion_layers.entry(node).or_default();
            for rule in rules.values() {
                layers.linked_transform |= rule.prop.is_transform();
                layers.transition_opacity |= rule.prop == ScrollTargetProp::Opacity;
                layers.linked_effect = true;
            }
        }

        if self.platform_views.is_empty() {
            for motion in &self.motions {
                let freeze = self
                    .get(motion.node)
                    .is_some_and(|node| node.flags.contains(NodeFlags::TRANSITION_SNAPSHOT));
                let layers = self.motion_layers.entry(motion.node).or_default();
                if freeze {
                    match motion.prop {
                        AnimProp::Opacity => layers.transition_opacity = true,
                        AnimProp::TranslateX
                        | AnimProp::TranslateY
                        | AnimProp::Rotate
                        | AnimProp::Scale => {
                            layers.linked_transform = true;
                        }
                        _ => {}
                    }
                }
                if let Some(Some(window)) = self.list_compositor_windows.get(&motion.node) {
                    for link in [
                        window.horizontal_thumb,
                        window.vertical_thumb,
                        window.sticky,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        self.motion_layers
                            .entry(link.node)
                            .or_default()
                            .linked_transform = true;
                    }
                }
            }
        }
    }

    pub(super) fn finish_compositor_record(&mut self) {
        let mut tracks = Vec::with_capacity(self.motion_layers.len());
        for motion in &self.motions {
            let Some(track) = self.compositor_track(motion) else {
                tracks.clear();
                break;
            };
            tracks.push(track);
        }
        tracks.sort_by_key(|track| track.layer.0);
        self.layers.tracks = if tracks.is_empty() {
            Arc::default()
        } else {
            Arc::from(tracks)
        };
    }

    fn compositor_track(&self, motion: &ActiveMotion) -> Option<CompositorTrack> {
        let layers = self.motion_layers.get(&motion.node)?;
        let curve = motion.simulation.compositor_curve()?;
        let (layer, prop, clamp, transform) = match motion.prop {
            AnimProp::Opacity => (
                layers.opacity?,
                CompositorProp::Opacity,
                Some([0.0, 255.0]),
                None,
            ),
            AnimProp::ScrollX | AnimProp::ScrollY => {
                if self.node(motion.node).ok()?.overscroll() != (0.0, 0.0) {
                    return None;
                }
                let (x, y) = self.scroll_limits(motion.node);
                let (limit, prop) = if motion.prop == AnimProp::ScrollX {
                    (x, CompositorProp::ScrollX)
                } else {
                    (y, CompositorProp::ScrollY)
                };
                (layers.scroll?, prop, Some([0.0, limit]), None)
            }
            AnimProp::TranslateX | AnimProp::TranslateY | AnimProp::Rotate | AnimProp::Scale => {
                let (layer, base) = layers.transform?;
                let prop = match motion.prop {
                    AnimProp::TranslateX => CompositorProp::TranslateX,
                    AnimProp::TranslateY => CompositorProp::TranslateY,
                    AnimProp::Rotate => CompositorProp::Rotate,
                    _ => CompositorProp::Scale,
                };
                (layer, prop, None, Some(base))
            }
            AnimProp::ShaderUniform(index) => {
                if self.node(motion.node).ok()?.first_child.is_some() {
                    return None;
                }
                let picture = self.pictures.get(&motion.node)?;
                if !picture
                    .commands()
                    .iter()
                    .any(|command| matches!(command, Command::ShaderEffect { .. }))
                {
                    return None;
                }
                let picture_id = child_picture_id(motion.node);
                let layer = self.layers.layers.iter().position(|layer| {
                    matches!(layer.kind, LayerKind::Picture { picture } if picture == picture_id)
                })?;
                (
                    LayerId(crate::cast::u32_from_usize(layer)),
                    CompositorProp::ShaderUniform(index),
                    None,
                    None,
                )
            }
            AnimProp::OverscrollX | AnimProp::OverscrollY => {
                if !matches!(curve, MotionCurve::Spring { .. })
                    || self
                        .list_compositor_windows
                        .get(&motion.node)
                        .is_some_and(Option::is_none)
                {
                    return None;
                }
                let prop = if motion.prop == AnimProp::OverscrollX {
                    CompositorProp::OverscrollX
                } else {
                    CompositorProp::OverscrollY
                };
                (layers.scroll?, prop, None, None)
            }
        };
        let mut track = CompositorTrack::new(
            motion.id,
            layer,
            prop,
            motion.started_ms,
            curve,
            clamp,
            transform,
        );
        track.freeze_subtree = matches!(
            prop,
            CompositorProp::Opacity
                | CompositorProp::TranslateX
                | CompositorProp::TranslateY
                | CompositorProp::Rotate
                | CompositorProp::Scale
        ) && self
            .node(motion.node)
            .ok()?
            .flags
            .contains(NodeFlags::TRANSITION_SNAPSHOT);
        let window = self.list_compositor_windows.get(&motion.node);
        let mut bindings = Vec::new();
        match prop {
            CompositorProp::ScrollX | CompositorProp::ScrollY => {
                if let Some(window) = window {
                    let window = window.as_ref()?;
                    let links = if prop == CompositorProp::ScrollY {
                        track.scroll_window = Some(window.bounds);
                        track.list_prefetch_edges = window.prefetch_edges;
                        [window.vertical_thumb, window.sticky]
                    } else {
                        [window.horizontal_thumb, None]
                    };
                    for link in links.into_iter().flatten() {
                        let (layer, transform) = self.motion_layers.get(&link.node)?.transform?;
                        let prop = if prop == CompositorProp::ScrollX {
                            ScrollTargetProp::TranslateX
                        } else {
                            ScrollTargetProp::TranslateY
                        };
                        bindings.push(ScrollBinding {
                            layer,
                            prop,
                            transform: Some(transform),
                            scroll: None,
                            curve: Arc::new(ScrollCurve::Linear {
                                factor: link.factor,
                                offset: link.offset,
                                maximum: link.maximum,
                            }),
                        });
                    }
                }
            }
            CompositorProp::OverscrollX | CompositorProp::OverscrollY => {
                let (x, y) = self.scroll_offset(motion.node).ok()?;
                track.overscroll_base = if prop == CompositorProp::OverscrollX {
                    x
                } else {
                    y
                };
                if prop == CompositorProp::OverscrollY
                    && let Some(Some(window)) = window
                {
                    let base = track.overscroll_base;
                    let limit = self.scroll_limits(motion.node).1;
                    let low = if base == 0.0 {
                        f32::NEG_INFINITY
                    } else {
                        window.bounds[0] - base
                    };
                    let high = if base == limit {
                        f32::INFINITY
                    } else {
                        window.bounds[1] - base
                    };
                    track.scroll_window = Some([low, high]);
                }
            }
            _ => {}
        }
        for (&target, rules) in &self.scroll_bindings {
            for rule in rules
                .values()
                .filter(|rule| matches!(rule.source, BindingSource::Scroll { node, channel } if node == motion.node && channel.prop() == prop))
            {
                let Some(layers) = self.motion_layers.get(&target) else {
                    continue;
                };
                if self.motions.iter().any(|motion| {
                    motion.node == target
                        && !matches!(
                            motion.prop,
                            AnimProp::ScrollX
                                | AnimProp::ScrollY
                                | AnimProp::OverscrollX
                                | AnimProp::OverscrollY
                        )
                }) {
                    return None;
                }
                let mut transform = None;
                let mut scroll = None;
                let layer = match rule.prop {
                    ScrollTargetProp::Opacity => layers.opacity?,
                    ScrollTargetProp::BlurSigma => layers.blur?,
                    ScrollTargetProp::BackdropBlurSigma => layers.backdrop?,
                    ScrollTargetProp::BackgroundColor => layers.fill?,
                    ScrollTargetProp::ShaderUniform(_) => self.scroll_shader_layer(target)?,
                    ScrollTargetProp::ScrollX | ScrollTargetProp::ScrollY => {
                        let channel = rule.prop.scroll_channel()?;
                        if self.list_compositor_windows.contains_key(&target)
                            || self
                                .scroll_thresholds
                                .values()
                                .any(|state| state.node == target && state.rule.source == channel)
                            || self.scroll_bindings.values().any(|rules| {
                                rules
                                    .values()
                                    .any(|rule| rule.source == (BindingSource::Scroll { node: target, channel }))
                            })
                            || self.motions.iter().any(|motion| motion.node == target)
                        {
                            return None;
                        }
                        let (_, limit) = self.scroll_channel(target, channel)?;
                        let (x, y) = self.overscroll(target).ok()?;
                        scroll = Some([limit, if channel == ScrollSource::X { x } else { y }]);
                        layers.scroll?
                    }
                    _ => {
                        let (layer, base) = layers.transform?;
                        transform = Some(base);
                        layer
                    }
                };
                bindings.push(ScrollBinding {
                    layer,
                    prop: rule.prop,
                    curve: Arc::clone(&rule.curve),
                    transform,
                    scroll,
                });
            }
        }
        track.scroll_bindings = bindings.into();
        track.threshold_window = self.scroll_threshold_range(motion.node, prop);
        track.sample(self.clock_ms)?;
        Some(track)
    }

    pub(super) fn list_scroll_prediction(&self, node: NodeId) -> Option<f32> {
        let motion = self
            .motions
            .iter()
            .find(|motion| motion.node == node && motion.prop == AnimProp::ScrollY)?;
        let curve = motion.simulation.compositor_curve()?;
        let predicted =
            crate::scene::list_scroll_prediction(curve, motion.started_ms, self.clock_ms);
        Some(predicted.clamp(0.0, self.scroll_limits(node).1))
    }

    /// Whether all active source motions still match the installed layer tracks.
    pub fn compositor_eligibility(
        &self,
        tracks: &[CompositorTrack],
        scroll_observed: impl Fn(NodeId) -> bool,
    ) -> crate::scene::CompositorEligibility {
        use crate::scene::CompositorBlockReason;

        for motion in &self.motions {
            if matches!(
                motion.prop,
                AnimProp::ScrollX
                    | AnimProp::ScrollY
                    | AnimProp::OverscrollX
                    | AnimProp::OverscrollY
            ) {
                if scroll_observed(motion.node) {
                    return Err(CompositorBlockReason::ScrollCallback {
                        node: motion.node.to_raw(),
                    });
                }
                for (&target, rules) in &self.scroll_bindings {
                    if scroll_observed(target)
                        && rules.values().any(|rule| {
                            rule.source.on_node(motion.node) && rule.prop.scroll_channel().is_some()
                        })
                    {
                        return Err(CompositorBlockReason::ScrollCallback {
                            node: target.to_raw(),
                        });
                    }
                }
            }
        }
        if !self.platform_views.is_empty() {
            return Err(CompositorBlockReason::PlatformViewPresent);
        }
        if self.needs_layout() || self.needs_paint() {
            return Err(CompositorBlockReason::PendingUiWork);
        }
        for motion in &self.motions {
            if !tracks.iter().any(|track| track.id == motion.id) {
                let node = motion.node.to_raw();
                return Err(
                    if self
                        .list_compositor_windows
                        .get(&motion.node)
                        .is_some_and(Option::is_none)
                    {
                        CompositorBlockReason::ListWindowNotSettled { node }
                    } else {
                        CompositorBlockReason::UnsupportedTrack { node }
                    },
                );
            }
        }
        if self.motions.len() != tracks.len() {
            return Err(CompositorBlockReason::PendingUiWork);
        }
        Ok(())
    }

    pub(super) fn scroll_shader_layer(&self, node: NodeId) -> Option<LayerId> {
        let picture = super::picture_id::self_picture_id(node);
        self.shader_effects.get(&node)?;
        let index = self.layers.layers.iter().position(
            |layer| matches!(layer.kind, LayerKind::Picture { picture: id } if id == picture),
        )?;
        Some(LayerId(crate::cast::u32_from_usize(index)))
    }

    /// Align the tree with the last frame actually shown before delivering input.
    pub fn commit_compositor_state(&mut self, snapshot: &CompositorSnapshot) {
        self.clock_ms = self.clock_ms.max(snapshot.target_ms);
        for sample in &snapshot.samples {
            if let Some(motion) = self.motions.iter().find(|motion| motion.id == sample.id) {
                let node = motion.node;
                let prop = motion.prop;
                self.apply_motion(node, prop, sample.value);
                if let Some(Some(window)) = self.list_compositor_windows.get(&node).copied() {
                    let links = match prop {
                        AnimProp::ScrollX => [window.horizontal_thumb, None],
                        AnimProp::ScrollY => [window.vertical_thumb, window.sticky],
                        _ => [None; 2],
                    };
                    for link in links.into_iter().flatten() {
                        let curve = ScrollCurve::Linear {
                            factor: link.factor,
                            offset: link.offset,
                            maximum: link.maximum,
                        };
                        if let crate::scene::ScrollValue::Scalar(value) = curve.sample(sample.value)
                        {
                            let (x, y) = if prop == AnimProp::ScrollX {
                                (value, 0.0)
                            } else {
                                (0.0, value)
                            };
                            let _ = self.set_transform(link.node, x, y, 0.0, 1.0);
                        }
                    }
                }
            }
        }
        self.sync_scroll_offsets();
    }
}

#[cfg(test)]
mod tests {
    use crate::motion::{AnimProp, Easing, Tween};
    use crate::scene::{CompositorProp, CompositorSnapshot, LayerKind, apply_compositor_samples};
    use crate::text::MonoShaper;

    use super::Tree;

    #[test]
    fn marked_transition_emits_tracks_at_zero_opacity_and_identity_transform()
    -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let root = tree.opacity(0)?;
        let child = tree.decorated([200, 0, 0, 255], 0.0)?;
        tree.set_size(child, 12.0, 12.0)?;
        tree.add_child(root, child)?;
        tree.set_transition_snapshot(root, true)?;
        tree.tick_motions(100.0);
        for (prop, to) in [(AnimProp::Opacity, 250.0), (AnimProp::TranslateY, 12.0)] {
            tree.animate(
                root,
                prop,
                Box::new(Tween {
                    from: 0.0,
                    to,
                    duration_ms: 400.0,
                    easing: Easing::Linear,
                }),
            );
        }
        tree.layout()?;
        tree.record_layers()?;
        let scene = tree.layer_tree();
        assert_eq!(scene.tracks.len(), 2);
        assert!(scene.tracks.iter().all(|track| track.freeze_subtree));
        assert!(
            scene
                .pictures
                .iter()
                .any(|picture| !picture.commands().is_empty())
        );
        Ok(())
    }

    #[test]
    fn grouped_transform_tracks_match_the_ui_tree_at_the_same_target() -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let transform = tree.transform()?;
        let rect = tree.sized(24.0, 16.0)?;
        tree.add_child(transform, rect)?;
        tree.set_transform(transform, 3.0, 7.0, 0.1, 1.0)?;
        tree.node_mut(transform)?.transform_mut().unwrap().pivot = Some((0.0, 0.0, 0.5, 0.5));
        tree.tick_motions(100.0);
        for (prop, from, to) in [
            (AnimProp::TranslateX, 3.0, 30.0),
            (AnimProp::Rotate, 0.1, 0.8),
            (AnimProp::Scale, 1.0, 2.0),
        ] {
            tree.animate(
                transform,
                prop,
                Box::new(Tween {
                    from,
                    to,
                    duration_ms: 400.0,
                    easing: Easing::EaseInOut,
                }),
            );
        }
        tree.layout()?;
        tree.record_layers()?;
        tree.clear_paint();
        let recorded = tree.layer_tree();
        assert_eq!(recorded.tracks.len(), 3);
        assert!(
            tree.compositor_eligibility(&recorded.tracks, |_| false)
                .is_ok()
        );

        let tracks = &recorded.tracks;
        let mut sampled_layers = recorded.layers.clone();
        for time in [108.333, 116.667, 250.0, 320.0] {
            let samples = tracks
                .iter()
                .map(|track| track.sample(time).unwrap())
                .collect::<Vec<_>>();
            assert!(apply_compositor_samples(
                &mut sampled_layers,
                tracks,
                &samples
            ));
            assert!(tree.tick_motions(time));
            tree.record_layers()?;
            let target = tracks[0].layer.0 as usize;
            assert_eq!(
                sampled_layers[target].kind,
                tree.layer_tree().layers[target].kind
            );
        }
        assert!(tracks[0].sample(500.0).is_none());
        assert!(!tree.tick_motions(500.0));
        assert_eq!(tree.take_motion_ends().len(), 3);
        assert!(tree.take_motion_ends().is_empty());
        Ok(())
    }

    #[test]
    fn a_scroll_edge_hands_off_and_commit_restores_only_visible_samples() -> Result<(), crate::Error>
    {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
        let scroll = tree.scroll()?;
        let body = tree.sized(40.0, 200.0)?;
        tree.set_max_size(scroll, 40.0, 40.0)?;
        tree.add_child(scroll, body)?;
        tree.layout()?;
        tree.tick_motions(10.0);
        tree.set_scroll_offset(scroll, 0.0, 90.0)?;
        tree.arm_fling(scroll, 0.0, -1.0)?;
        tree.record_layers()?;
        tree.clear_paint();

        let mut recorded = tree.layer_tree();
        let [track] = recorded.tracks.as_ref() else {
            panic!("one scroll track")
        };
        assert_eq!(track.prop, CompositorProp::ScrollY);
        assert!(
            tree.compositor_eligibility(&recorded.tracks, |_| false)
                .is_ok()
        );
        assert!(
            tree.compositor_eligibility(&recorded.tracks, |node| node == scroll)
                .is_err()
        );
        let target_ms = 10.0 + 1000.0 / 120.0;
        let sample = track.sample(target_ms).unwrap();
        assert!(apply_compositor_samples(
            &mut recorded.layers,
            &recorded.tracks,
            &[sample]
        ));
        assert_eq!(
            recorded.layers[track.layer.0 as usize].kind,
            LayerKind::Offset {
                x: 0.0,
                y: -sample.value
            }
        );
        tree.commit_compositor_state(&CompositorSnapshot {
            target_ms,
            samples: vec![sample],
        });
        assert_eq!(tree.scroll_offset(scroll)?, (0.0, sample.value));
        assert!(
            tree.take_motion_ends().is_empty(),
            "interrupt is not completion"
        );

        assert!(track.sample(100.0).is_none());
        tree.tick_motions(100.0);
        assert_eq!(tree.take_motion_ends(), vec![(scroll, AnimProp::ScrollY)]);
        assert!(tree.take_motion_ends().is_empty());
        Ok(())
    }
}
