//! Application scroll rules and their UI-side state.

use std::sync::Arc;

use crate::Error;
use crate::motion::AnimProp;
use crate::scene::{
    CompositorProp, ScrollCurve, ScrollSource, ScrollTargetProp, ScrollThreshold, ScrollValue,
};

use super::node::TransformGeom;
use super::{NodeId, Tree};

pub(super) struct ThresholdState {
    pub node: NodeId,
    pub rule: ScrollThreshold,
    pub active: Option<bool>,
}

impl Tree {
    pub fn add_scroll_threshold(
        &mut self,
        node: NodeId,
        rule: ScrollThreshold,
    ) -> Result<u32, Error> {
        self.overscroll(node)?;
        if !rule.is_valid() {
            return Err(Error::BadNumber);
        }
        self.next_scroll_rule = self
            .next_scroll_rule
            .checked_add(1)
            .ok_or(Error::BadNumber)?;
        let id = self.next_scroll_rule;
        self.scroll_thresholds.insert(
            id,
            ThresholdState {
                node,
                rule,
                active: None,
            },
        );
        self.paint_layer_value();
        Ok(id)
    }

    pub fn remove_scroll_threshold(&mut self, id: u32) {
        if self.scroll_thresholds.remove(&id).is_some() {
            self.paint_layer_value();
        }
    }

    pub(super) fn scroll_channel(&self, node: NodeId, source: ScrollSource) -> Option<(f32, f32)> {
        let geom = self.get(node)?.scroll_geom()?;
        let limits = self.scroll_limits(node);
        Some(match source {
            ScrollSource::X => (geom.offset_x, limits.0),
            ScrollSource::Y => (geom.offset_y, limits.1),
            ScrollSource::OverscrollX => (geom.overscroll_x, 0.0),
            ScrollSource::OverscrollY => (geom.overscroll_y, 0.0),
        })
    }

    /// Initializes rules after layout and returns each edge once.
    pub fn update_scroll_thresholds(&mut self) -> Vec<(u32, bool)> {
        let mut changes = Vec::new();
        let mut rules = std::mem::take(&mut self.scroll_thresholds);
        rules.retain(|&id, state| {
            let Some((value, limit)) = self.scroll_channel(state.node, state.rule.source) else {
                return false;
            };
            let active = state.rule.evaluate(value, limit, state.active);
            if state.active != Some(active) {
                state.active = Some(active);
                changes.push((id, active));
            }
            true
        });
        self.scroll_thresholds = rules;
        changes
    }

    pub(super) fn scroll_threshold_range(
        &self,
        node: NodeId,
        prop: CompositorProp,
    ) -> Option<[f32; 2]> {
        let mut range: Option<[f32; 2]> = None;
        for state in self.scroll_thresholds.values() {
            if state.node != node || state.rule.source.prop() != prop {
                continue;
            }
            let (Some(active), Some((_, limit))) =
                (state.active, self.scroll_channel(node, state.rule.source))
            else {
                // The UI must initialize newly registered rules before installing tracks.
                return Some([0.0, 0.0]);
            };
            let [low, high] = state.rule.stable_range(limit, active);
            range = Some(range.map_or([low, high], |[a, b]| [low.max(a), high.min(b)]));
        }
        range
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingSource {
    Scroll { node: NodeId, channel: ScrollSource },
    BackProgress,
}

impl BindingSource {
    pub(super) fn on_node(self, node: NodeId) -> bool {
        matches!(self, Self::Scroll { node: source, .. } if source == node)
    }
}

pub(super) struct NodeScrollBinding {
    pub source: BindingSource,
    pub prop: ScrollTargetProp,
    pub curve: Arc<ScrollCurve>,
}

impl Tree {
    pub fn add_scroll_binding(
        &mut self,
        source: NodeId,
        channel: ScrollSource,
        target: NodeId,
        prop: ScrollTargetProp,
        curve: ScrollCurve,
    ) -> Result<u32, Error> {
        self.overscroll(source)?;
        self.add_visual_binding(
            BindingSource::Scroll {
                node: source,
                channel,
            },
            target,
            prop,
            curve,
        )
    }

    pub fn bind_back_progress_opacity(
        &mut self,
        target: NodeId,
        curve: crate::scene::PiecewiseMap,
    ) -> Result<u32, Error> {
        self.add_visual_binding(
            BindingSource::BackProgress,
            target,
            ScrollTargetProp::Opacity,
            ScrollCurve::Scalar(curve),
        )
    }

    pub fn set_back_progress(&mut self, progress: f32) {
        if !progress.is_finite() {
            return;
        }
        let progress = progress.clamp(0.0, 1.0);
        if progress != self.back_progress {
            self.back_progress = progress;
            self.paint_layer_value();
        }
    }

    fn add_visual_binding(
        &mut self,
        source: BindingSource,
        target: NodeId,
        prop: ScrollTargetProp,
        curve: ScrollCurve,
    ) -> Result<u32, Error> {
        self.node(target)?;
        if !curve.is_valid()
            || matches!(curve, ScrollCurve::Color(_)) != (prop == ScrollTargetProp::BackgroundColor)
        {
            return Err(Error::BadNumber);
        }
        if let Some(channel) = prop.scroll_channel() {
            let BindingSource::Scroll { node: source, .. } = source else {
                return Err(Error::WrongKind);
            };
            self.overscroll(target)?;
            let anim = match channel {
                ScrollSource::X => AnimProp::ScrollX,
                _ => AnimProp::ScrollY,
            };
            if !self.axis_of(target).allows(anim)
                || source == target
                || self.scroll_bindings.get(&source).is_some_and(|rules| {
                    rules
                        .values()
                        .any(|rule| rule.prop.scroll_channel().is_some())
                })
                || self.scroll_bindings.values().any(|rules| {
                    rules.values().any(|rule| {
                        rule.source.on_node(target) && rule.prop.scroll_channel().is_some()
                    })
                })
            {
                return Err(Error::WrongKind);
            }
        }
        // A channel has one owner. Uniform and axis-specific scale overlap.
        if self.scroll_bindings.get(&target).is_some_and(|rules| {
            rules.values().any(|rule| {
                rule.prop == prop
                    || matches!(
                        (rule.prop, prop),
                        (
                            ScrollTargetProp::Scale,
                            ScrollTargetProp::ScaleX | ScrollTargetProp::ScaleY
                        ) | (
                            ScrollTargetProp::ScaleX | ScrollTargetProp::ScaleY,
                            ScrollTargetProp::Scale
                        )
                    )
            })
        }) {
            return Err(Error::WrongKind);
        }
        self.next_scroll_rule = self
            .next_scroll_rule
            .checked_add(1)
            .ok_or(Error::BadNumber)?;
        let id = self.next_scroll_rule;
        self.scroll_bindings.entry(target).or_default().insert(
            id,
            NodeScrollBinding {
                source,
                prop,
                curve: Arc::new(curve),
            },
        );
        self.repaint_ancestors(target);
        Ok(id)
    }

    pub fn remove_scroll_binding(&mut self, id: u32) {
        let target = self
            .scroll_bindings
            .iter_mut()
            .find_map(|(&target, rules)| rules.remove(&id).map(|_| target));
        if let Some(target) = target {
            if self
                .scroll_bindings
                .get(&target)
                .is_some_and(|rules| rules.is_empty())
            {
                self.scroll_bindings.remove(&target);
            }
            self.repaint_ancestors(target);
        }
    }

    pub(super) fn forget_scroll_rules(&mut self, node: NodeId) {
        self.scroll_thresholds.retain(|_, state| state.node != node);
        self.scroll_bindings.remove(&node);
        let stale: Vec<_> = self
            .scroll_bindings
            .values()
            .flat_map(|rules| rules.iter())
            .filter_map(|(&id, rule)| rule.source.on_node(node).then_some(id))
            .collect();
        for id in stale {
            self.remove_scroll_binding(id);
        }
    }

    pub(super) fn binding_values(
        &self,
        target: NodeId,
    ) -> impl Iterator<Item = (ScrollTargetProp, ScrollValue)> + '_ {
        self.scroll_bindings
            .get(&target)
            .into_iter()
            .flat_map(|rules| rules.values())
            .filter_map(|rule| {
                let value = match rule.source {
                    BindingSource::Scroll { node, channel } => {
                        self.scroll_channel(node, channel)?.0
                    }
                    BindingSource::BackProgress => self.back_progress,
                };
                Some((rule.prop, rule.prop.evaluate(&rule.curve, value)))
            })
    }

    pub(super) fn effective_transform(&self, id: NodeId) -> Option<TransformGeom> {
        let geometry = self.get(id)?.transform();
        if !self.scroll_bindings.contains_key(&id) {
            return Some(geometry);
        }
        let mut transform = geometry.compositor_transform(0.0, 0.0, 0.0, 0.0);
        for (prop, value) in self.binding_values(id) {
            if let ScrollValue::Scalar(value) = value {
                prop.apply_transform(&mut transform, value);
            }
        }
        Some(TransformGeom {
            translate_x: transform.translate[0],
            translate_y: transform.translate[1],
            rotate_rad: transform.rotate_rad,
            scale_x: transform.scale[0],
            scale_y: transform.scale[1],
            pivot: geometry.pivot,
        })
    }

    /// Reconcile linked offsets before list measurement or input sees the tree.
    pub fn sync_scroll_offsets(&mut self) {
        let offsets: Vec<_> = self
            .scroll_bindings
            .keys()
            .flat_map(|&target| {
                self.binding_values(target)
                    .filter_map(move |(prop, value)| {
                        let channel = prop.scroll_channel()?;
                        let ScrollValue::Scalar(value) = value else {
                            return None;
                        };
                        let anim = if channel == ScrollSource::X {
                            AnimProp::ScrollX
                        } else {
                            AnimProp::ScrollY
                        };
                        Some((target, anim, value))
                    })
            })
            .collect();
        for (target, prop, value) in offsets {
            self.apply_motion(target, prop, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::motion::{AnimProp, Easing, Tween};
    use crate::scene::{CompositorProp, ScrollSource, ScrollThreshold};
    use crate::text::MonoShaper;

    use super::{super::compositor_motion::ListCompositorWindow, Tree};

    #[test]
    fn threshold_crossings_are_discrete_and_keep_their_state_inside_the_band()
    -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
        let scroll = tree.scroll()?;
        let content = tree.sized(40.0, 1000.0)?;
        tree.set_max_size(scroll, 40.0, 40.0)?;
        tree.add_child(scroll, content)?;
        tree.layout()?;
        let id = tree.add_scroll_threshold(
            scroll,
            ScrollThreshold {
                source: ScrollSource::Y,
                value: 300.0,
                from_end: false,
                hysteresis: 4.0,
            },
        )?;
        let end = tree.add_scroll_threshold(
            scroll,
            ScrollThreshold {
                source: ScrollSource::Y,
                value: -200.0,
                from_end: true,
                hysteresis: 0.0,
            },
        )?;
        assert_eq!(tree.update_scroll_thresholds(), [(id, false), (end, false)]);
        for (y, changes) in [
            (300.0, vec![]),
            (304.0, vec![(id, true)]),
            (303.0, vec![]),
            (296.0, vec![]),
            (295.0, vec![(id, false)]),
            (760.0, vec![(id, true), (end, true)]),
            (780.0, vec![]),
            (759.0, vec![(end, false)]),
        ] {
            tree.set_scroll_offset(scroll, 0.0, y)?;
            assert_eq!(tree.update_scroll_thresholds(), changes);
        }
        Ok(())
    }

    #[test]
    fn list_and_discrete_thresholds_both_limit_takeover() -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::default());
        tree.set_viewport(0.0, 0.0, 40.0, 40.0)?;
        let scroll = tree.scroll()?;
        let content = tree.sized(40.0, 700.0)?;
        tree.set_max_size(scroll, 40.0, 40.0)?;
        tree.add_child(scroll, content)?;
        tree.layout()?;
        tree.tick_motions(100.0);
        tree.set_scroll_offset(scroll, 0.0, 100.0)?;
        let id = tree.add_scroll_threshold(
            scroll,
            ScrollThreshold {
                source: ScrollSource::Y,
                value: 300.0,
                from_end: false,
                hysteresis: 0.0,
            },
        )?;
        tree.list_compositor_windows.insert(
            scroll,
            Some(ListCompositorWindow {
                bounds: [70.0, 350.0],
                prefetch_edges: [None; 2],
                horizontal_thumb: None,
                vertical_thumb: None,
                sticky: None,
            }),
        );
        tree.animate(
            scroll,
            AnimProp::ScrollY,
            Box::new(Tween {
                from: 100.0,
                to: 500.0,
                duration_ms: 400.0,
                easing: Easing::Linear,
            }),
        );
        assert_eq!(tree.update_scroll_thresholds(), [(id, false)]);
        tree.record_layers()?;
        let installed = tree.layer_tree();
        let [track] = installed.tracks.as_ref() else {
            panic!("expected one scroll track")
        };
        assert_eq!(track.prop, CompositorProp::ScrollY);
        assert_eq!(track.sample(299.0).map(|sample| sample.value), Some(299.0));
        assert!(track.sample(300.0).is_none());
        tree.tick_motions(300.0);
        assert_eq!(tree.update_scroll_thresholds(), [(id, true)]);
        tree.record_layers()?;
        let installed = tree.layer_tree();
        let [track] = installed.tracks.as_ref() else {
            panic!("expected reinstalled track")
        };
        assert!(track.sample(301.0).is_some());
        assert!(track.sample(350.0).is_none());
        Ok(())
    }
}
