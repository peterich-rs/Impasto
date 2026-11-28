//! Tracks picture and layer changes without rebuilding unchanged subtrees.

use std::sync::Arc;

use super::{Node, NodeId, Role, Tree, rect_paint};

impl Tree {
    /// The last retained scene changes, relative to the preceding layer snapshot.
    pub fn take_scene_delta(&mut self) -> Option<crate::scene::SceneDelta> {
        self.retained_layers.delta.take()
    }

    pub(in crate::tree) fn invalidate_layer_delta(&mut self) {
        self.retained_layers.invalidate();
    }

    pub(in crate::tree) fn uses_paint_slot(&self, id: NodeId) -> bool {
        self.retained_layers.paint_nodes.contains(&id)
            && self.get(id).is_some_and(slot_compatible)
            && !self.scroll_bindings.contains_key(&id)
    }

    pub(in crate::tree) fn forget_paint_slot(&mut self, id: NodeId) {
        self.retained_layers.paint_nodes.remove(&id);
    }

    pub(in crate::tree) fn update_rect_slot(&mut self, id: NodeId) -> bool {
        self.stage_repaint(id, super::super::RepaintReason::STYLE_PAINT);
        let Some(node) = self.get(id) else {
            return false;
        };
        if !slot_compatible(node) || self.scroll_bindings.contains_key(&id) {
            return false;
        }
        if let Some(&layer) = self.retained_layers.paint.get(&id)
            && let Some(crate::scene::LayerKind::Paint(previous)) = self
                .layers
                .layers
                .get(layer.0 as usize)
                .map(|layer| &layer.kind)
        {
            let slot = rect_slot(node, previous.bounds);
            if node.role.contains(Role::CLIP)
                && (previous.radius != slot.radius || previous.corners != slot.corners)
            {
                let Some((clip, shape)) = self.clip_slot(id) else {
                    return false;
                };
                if !self.queue_layer_patch(clip, shape) {
                    return false;
                }
            }
            return self.queue_layer_patch(layer, crate::scene::LayerKind::Paint(slot));
        }
        if node.fill().is_none() && node.stroke_width() <= 0.0 {
            return false;
        }
        if self.laid_out && self.retained_layers.paint_nodes.insert(id) {
            self.repaint_ancestors(id);
            return true;
        }
        false
    }

    pub(in crate::tree) fn update_clip_radius(&mut self, id: NodeId) -> bool {
        self.stage_repaint(id, super::super::RepaintReason::CLIP_OR_RADIUS);
        self.get(id).is_some_and(|node| {
            node.fill().is_none()
                && node.stroke_width() <= 0.0
                && node.gradient().is_none()
                && node.shadow().is_none()
                && node.box_shadows().is_empty()
                && node
                    .paint
                    .as_ref()
                    .is_none_or(|paint| paint.outline_width <= 0.0)
        }) && self
            .clip_slot(id)
            .is_some_and(|(layer, kind)| self.queue_layer_patch(layer, kind))
    }

    fn clip_slot(&self, id: NodeId) -> Option<(crate::scene::LayerId, crate::scene::LayerKind)> {
        let &(layer, bounds) = self.retained_layers.clips.get(&id)?;
        let node = self.get(id)?;
        Some((
            layer,
            clip_kind(bounds, node.corner_radius(), node.corners()),
        ))
    }

    pub(in crate::tree) fn update_transform_slot(&mut self, id: NodeId) -> bool {
        self.stage_repaint(id, super::super::RepaintReason::TRANSFORM_OR_OPACITY);
        let Some(&(layer, [x, y, w, h])) = self.retained_layers.transforms.get(&id) else {
            return false;
        };
        let Some(node) = self.get(id) else {
            return false;
        };
        let geom = node.transform().placed(x, y, w, h);
        self.canvas_geometry_changed();
        self.queue_layer_patch(
            layer,
            crate::scene::LayerKind::Transform {
                translate_x: geom.translate_x,
                translate_y: geom.translate_y,
                rotate_rad: geom.rotate_rad,
                scale_x: geom.scale_x,
                scale_y: geom.scale_y,
            },
        )
    }

    pub(in crate::tree) fn update_opacity_slot(&mut self, id: NodeId, alpha: u8) -> bool {
        self.stage_repaint(id, super::super::RepaintReason::TRANSFORM_OR_OPACITY);
        let Some(&layer) = self.retained_layers.opacity.get(&id) else {
            return false;
        };
        let Some(crate::scene::LayerKind::Opacity { x, y, w, h, .. }) = self
            .layers
            .layers
            .get(layer.0 as usize)
            .map(|layer| layer.kind.clone())
        else {
            return false;
        };
        self.queue_layer_patch(
            layer,
            crate::scene::LayerKind::Opacity { alpha, x, y, w, h },
        )
    }

    pub(in crate::tree) fn queue_picture_patch(&mut self, owner: NodeId) -> bool {
        if !self.can_patch_scene() || !self.retained_layers.owners.contains_key(&owner) {
            return false;
        }
        self.retained_layers.picture_patches.insert(owner);
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.needs_paint = true;
        true
    }

    fn can_patch_scene(&self) -> bool {
        !self.retained_layers.invalid
            && !self.needs_layout()
            && self.motions.is_empty()
            && self.scroll_bindings.is_empty()
            && self.platform_views.is_empty()
    }

    fn queue_layer_patch(
        &mut self,
        layer: crate::scene::LayerId,
        value: crate::scene::LayerKind,
    ) -> bool {
        if !self.can_patch_scene() {
            return false;
        }
        self.retained_layers.pending.insert(layer, value);
        self.layer_epoch = self.layer_epoch.wrapping_add(1);
        self.needs_paint = true;
        true
    }

    pub(super) fn apply_layer_patches(&mut self) -> bool {
        if self.retained_layers.invalid
            || (self.retained_layers.pending.is_empty()
                && self.retained_layers.picture_patches.is_empty())
        {
            return false;
        }
        self.picture_writes = 0;
        self.layer_record_visits = 0;
        let mut updated = Vec::new();
        let mut changed_layers = Vec::with_capacity(
            self.retained_layers.pending.len() + self.retained_layers.picture_patches.len(),
        );
        let mut owners = std::mem::take(&mut self.retained_layers.picture_patches);
        for node in owners.drain() {
            let owner = self.retained_layers.owners[&node];
            let commands = self.record_owner(node, owner.origin);
            self.clear_repaint(node);
            let prior = &self.layers.pictures[owner.picture];
            if prior.commands().as_ref() == commands.as_slice() {
                continue;
            }
            self.picture_writes = self.picture_writes.saturating_add(commands.len());
            let picture = Arc::new(crate::scene::Picture::new(
                prior.id,
                self.layer_epoch,
                prior.origin,
                Arc::from(commands),
            ));
            updated.push(crate::scene::PictureUpdate {
                index: owner.picture,
                id: prior.id,
            });
            self.commit_picture_repaint(
                node,
                super::super::PaintPath::PicturePatch,
                picture.id,
                picture.commands().len(),
            );
            self.layers.pictures[owner.picture] = Arc::clone(&picture);
            self.pictures.insert(node, picture);
            changed_layers.push(owner.layer);
        }
        self.retained_layers.picture_patches = owners;
        let mut pending = std::mem::take(&mut self.retained_layers.pending);
        for (id, kind) in pending.drain() {
            let layer = &mut self.layers.layers[id.0 as usize];
            if layer.kind != kind {
                layer.kind = kind;
                changed_layers.push(id);
                if let Some(LayerKey::Node(node, _)) = self.retained_layers.keys.get(id.0 as usize)
                {
                    self.commit_repaint(*node, super::super::PaintPath::FastLayerPatch, None, 0);
                }
            }
        }
        self.retained_layers.pending = pending;
        changed_layers.sort_unstable_by_key(|layer| layer.0);
        let changed: Arc<[_]> = changed_layers.into();
        self.retained_layers.delta = Some(crate::scene::SceneDelta {
            base_epoch: self.layers.epoch,
            removed_or_changed: Arc::clone(&changed),
            inserted_or_changed: changed,
            changes: if updated.is_empty() {
                crate::scene::SceneChanges::LayerValues
            } else {
                updated.sort_unstable_by_key(|update| update.index);
                crate::scene::SceneChanges::Pictures(updated.into())
            },
        });
        self.layers.epoch = self.layer_epoch;
        true
    }
}

#[derive(Default)]
pub(in crate::tree) struct RetainedLayers {
    pub(super) invalid: bool,
    paint_nodes: std::collections::HashSet<NodeId>,
    pub(super) paint: std::collections::HashMap<NodeId, crate::scene::LayerId>,
    pub(super) transforms: std::collections::HashMap<NodeId, (crate::scene::LayerId, [f32; 4])>,
    pub(super) opacity: std::collections::HashMap<NodeId, crate::scene::LayerId>,
    pub(super) clips: std::collections::HashMap<NodeId, (crate::scene::LayerId, [f32; 4])>,
    pending: std::collections::HashMap<crate::scene::LayerId, crate::scene::LayerKind>,
    pub(super) delta: Option<crate::scene::SceneDelta>,
    pub(super) keys: Vec<LayerKey>,
    pub(super) spare: RecordedScene,
    pub(super) owners: std::collections::HashMap<NodeId, RecordedOwner>,
    picture_patches: std::collections::HashSet<NodeId>,
}

#[derive(Clone, Copy)]
pub(super) struct RecordedOwner {
    pub(super) origin: (f32, f32),
    pub(super) picture: usize,
    pub(super) layer: crate::scene::LayerId,
}

impl RetainedLayers {
    fn invalidate(&mut self) {
        self.invalid = true;
        self.pending.clear();
        self.picture_patches.clear();
    }

    pub(super) fn begin(&mut self) {
        self.invalidate();
        self.paint.clear();
        self.transforms.clear();
        self.opacity.clear();
        self.clips.clear();
        self.owners.clear();
        self.delta = None;
    }
}

/// Dense layer addresses may shift when siblings are mounted or moved.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LayerKey {
    Node(NodeId, std::mem::Discriminant<crate::scene::LayerKind>),
    Picture(crate::scene::PictureId),
}

#[derive(Default)]
pub(super) struct RecordedScene {
    pub(super) layers: Vec<crate::scene::Layer>,
    pub(super) pictures: Vec<Arc<crate::scene::Picture>>,
    pub(super) keys: Vec<LayerKey>,
}

impl RecordedScene {
    pub(super) fn changes(
        &mut self,
        next: &crate::scene::LayerTree,
        keys: &[LayerKey],
    ) -> Option<crate::scene::SceneDelta> {
        use crate::scene::{Layer, LayerId};
        use std::collections::HashMap;

        if self.layers.is_empty() {
            return None;
        }
        let mut previous = HashMap::with_capacity(self.keys.len());
        for (index, &key) in self.keys.iter().enumerate() {
            if previous.insert(key, index).is_some() {
                return None;
            }
        }
        let previous_pictures: HashMap<_, _> = self
            .pictures
            .drain(..)
            .map(|picture| (picture.id, picture))
            .collect();
        let changed_pictures: std::collections::HashSet<_> = next
            .pictures
            .iter()
            .filter(|picture| {
                previous_pictures
                    .get(&picture.id)
                    .is_none_or(|prior| !Arc::ptr_eq(prior, picture))
            })
            .map(|picture| picture.id)
            .collect();
        let layer_id = |index| LayerId(crate::cast::u32_from_usize(index));
        let links = |layer: &Layer, keys: &[LayerKey]| {
            [layer.parent, layer.first_child, layer.next].map(|id| id.map(|id| keys[id.0 as usize]))
        };
        // If two runs swap places, the interior of each run may keep its
        // original links even though their overlapping pixels change order.
        let matched: Vec<_> = keys.iter().map(|key| previous.get(key).copied()).collect();
        let mut reordered = vec![false; keys.len()];
        let mut highest = 0;
        for (index, prior) in matched.iter().copied().enumerate() {
            if let Some(prior) = prior {
                reordered[index] = prior < highest;
                highest = highest.max(prior);
            }
        }
        let mut lowest = usize::MAX;
        for (index, prior) in matched.into_iter().enumerate().rev() {
            if let Some(prior) = prior {
                reordered[index] |= prior > lowest;
                lowest = lowest.min(prior);
            }
        }
        let mut before = Vec::new();
        let mut after = Vec::new();
        for (index, &key) in keys.iter().enumerate() {
            let Some(prior) = previous.remove(&key) else {
                after.push(layer_id(index));
                continue;
            };
            let old = &self.layers[prior];
            let new = &next.layers[index];
            if old.kind != new.kind
                || reordered[index]
                || links(old, &self.keys) != links(new, keys)
                || matches!(key, LayerKey::Picture(picture) if changed_pictures.contains(&picture))
            {
                before.push(layer_id(prior));
                after.push(layer_id(index));
            }
        }
        before.extend(previous.into_values().map(layer_id));
        before.sort_unstable_by_key(|id| id.0);
        Some(crate::scene::SceneDelta {
            base_epoch: next.epoch,
            removed_or_changed: before.into(),
            inserted_or_changed: after.into(),
            changes: crate::scene::SceneChanges::Topology,
        })
    }
}

pub(super) fn clip_kind(
    bounds: [f32; 4],
    radius: f32,
    corners: Option<crate::scene::CornerShape>,
) -> crate::scene::LayerKind {
    if let Some(corners) = corners {
        crate::scene::LayerKind::CornerClip { bounds, corners }
    } else {
        let [x, y, w, h] = bounds;
        crate::scene::LayerKind::Clip {
            x,
            y,
            w,
            h,
            corner_radius: radius,
        }
    }
}

fn slot_compatible(node: &Node) -> bool {
    node.gradient().is_none() && node.shadow().is_none() && node.box_shadows().is_empty()
}

pub(super) fn rect_slot(node: &Node, bounds: [f32; 4]) -> crate::scene::PaintSlot {
    let paint = rect_paint(node);
    crate::scene::PaintSlot {
        bounds,
        radius: node.corner_radius(),
        corners: node.corners(),
        color: paint.fill.unwrap_or([0; 4]),
        stroke_width: paint.stroke_width,
        stroke_color: paint.stroke_color,
    }
}

#[cfg(test)]
mod retained_tests {
    use crate::scene::SceneChanges;
    use crate::text::MonoShaper;
    use crate::tree::Tree;

    #[test]
    fn changed_picture_owner_records_only_its_own_commands() -> Result<(), crate::Error> {
        let mut tree = Tree::new(MonoShaper::new());
        let root = tree.stack()?;
        let first = tree.text("first", 16.0, [0, 0, 0, 255])?;
        let second = tree.text("second", 16.0, [0, 0, 0, 255])?;
        for row in [first, second] {
            tree.add_child(root, row)?;
            tree.set_picture_owner(row, true)?;
        }
        tree.layout()?;
        tree.record_layers()?;
        tree.clear_paint();
        let before = tree.layer_tree();
        tree.set_color(second, [0, 0, 255, 255])?;
        tree.record_layers()?;
        let delta = tree.take_scene_delta().expect("picture delta");
        let SceneChanges::Pictures(updates) = delta.changes else {
            panic!("expected a picture replacement");
        };
        assert_eq!(updates.len(), 1);
        assert_eq!(delta.base_epoch, before.epoch);
        assert_eq!(tree.layer_record_visits(), 1);
        let after = tree.layer_tree();
        assert!(std::sync::Arc::ptr_eq(
            &before.pictures[0],
            &after.pictures[0]
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &before.pictures[1],
            &after.pictures[1]
        ));

        let third = tree.text("third", 16.0, [0, 0, 0, 255])?;
        tree.add_child(root, third)?;
        tree.layout()?;
        tree.record_layers()?;
        let delta = tree.take_scene_delta().expect("topology delta");
        assert_eq!(delta.changes, SceneChanges::Topology);
        Ok(())
    }
}
