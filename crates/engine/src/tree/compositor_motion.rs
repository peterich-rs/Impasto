//! Picks motions whose source state can sleep until a compositor handoff.

use std::sync::Arc;

use crate::motion::{AnimProp, MotionCurve};
use crate::scene::{
    CompositorProp, CompositorSnapshot, CompositorTrack, CompositorTransform, LayerId,
};

use super::{ActiveMotion, NodeId, Tree};

/// Links a motion's node to the actual outer/content layers emitted by recording.
#[derive(Default)]
pub(super) struct MotionLayers {
    pub(super) transform: Option<(LayerId, CompositorTransform)>,
    pub(super) opacity: Option<LayerId>,
    pub(super) scroll: Option<LayerId>,
}

impl Tree {
    pub(super) fn begin_compositor_record(&mut self) {
        self.motion_layers.clear();
        if self.platform_views.is_empty() {
            for motion in &self.motions {
                self.motion_layers.entry(motion.node).or_default();
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
                if !matches!(curve, MotionCurve::Friction { .. })
                    || motion.snap
                    || self.node(motion.node).ok()?.overscroll() != (0.0, 0.0)
                {
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
            AnimProp::OverscrollX | AnimProp::OverscrollY => return None,
        };
        let track = CompositorTrack::new(
            motion.id,
            layer,
            prop,
            motion.started_ms,
            curve,
            clamp,
            transform,
        );
        track.sample(self.clock_ms)?;
        Some(track)
    }

    /// Whether all active source motions still match the installed layer tracks.
    pub fn compositor_motions_ready(
        &self,
        tracks: &[CompositorTrack],
        scroll_observed: impl Fn(NodeId) -> bool,
    ) -> bool {
        !self.needs_layout()
            && !self.needs_paint()
            && !self.pending_long_press()
            && self.platform_views.is_empty()
            && self.motions.len() == tracks.len()
            && self.motions.iter().all(|motion| {
                tracks.iter().any(|track| {
                    track.id == motion.id
                        && (!track.prop.is_scroll() || !scroll_observed(motion.node))
                })
            })
    }

    /// Align the tree with the last frame actually shown before delivering input.
    pub fn commit_compositor_state(&mut self, snapshot: &CompositorSnapshot) {
        self.clock_ms = self.clock_ms.max(snapshot.target_ms);
        for sample in &snapshot.samples {
            if let Some(motion) = self.motions.iter().find(|motion| motion.id == sample.id) {
                self.apply_motion(motion.node, motion.prop, sample.value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::motion::{AnimProp, Easing, Tween};
    use crate::scene::{CompositorProp, CompositorSnapshot, LayerKind, apply_compositor_samples};
    use crate::text::MonoShaper;

    use super::Tree;

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
        assert!(tree.compositor_motions_ready(&recorded.tracks, |_| false));

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
        assert!(tree.compositor_motions_ready(&recorded.tracks, |_| false));
        assert!(!tree.compositor_motions_ready(&recorded.tracks, |node| node == scroll));
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
