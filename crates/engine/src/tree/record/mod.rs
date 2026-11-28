//! 从已布局的树录制合成层和绘制命令。

use std::sync::Arc;

use crate::scene::{Command, DrawList};

use super::node::{Node, Role};
use super::{NodeId, Tree};
use crate::Error;

mod flat;
mod layers;
mod pass;
mod retained;

use flat::translate_command;
pub(super) use retained::RetainedLayers;
use retained::{LayerKey, RecordedOwner};

/// One node in the iterative layer sweep. `expanded` means its children are already stacked.
#[derive(Clone, Copy)]
pub(super) struct SweepFrame {
    id: NodeId,
    expanded: bool,
}

/// Properties of strict descendants that affect the owner's recording.
#[derive(Clone, Copy, Default)]
pub(super) struct SubtreeLayers {
    pub(super) effects: bool,
    pub(super) picture_owners: bool,
}

impl SubtreeLayers {
    pub(super) fn requires_clip_layer(self) -> bool {
        // A descendant's picture boundary closes the current command run.
        self.effects || self.picture_owners
    }
}

/// Axis-aligned clip in paint space. `None` means a transform ancestor made culling unsafe.
#[derive(Clone, Copy)]
pub(super) struct PaintClip {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Parent origin, scroll offset, and clip for one `record_node` visit.
#[derive(Clone, Copy)]
struct RecordAt {
    parent: (f32, f32),
    offset: (f32, f32),
    space: (f32, f32),
    clip: Option<PaintClip>,
    descend: bool,
}

/// Coordinate frame of the run currently being captured.
#[derive(Clone, Copy)]
pub(super) struct RunScope {
    owner: NodeId,
    /// Picture origin. Content commands are relative to this point.
    base: (f32, f32),
    /// Added to captured commands when writing the flat list.
    mirror: (f32, f32),
    /// Next segment index for `owner` in this frame.
    segment: usize,
}

/// In-progress run. `scope` is `None` outside [`Tree::record_layers`].
#[derive(Default)]
pub(super) struct RunState {
    scope: Option<RunScope>,
    commands: Vec<Command>,
    first: Option<NodeId>,
}

struct FlatList<'a>(Option<&'a mut Vec<Command>>);

impl FlatList<'_> {
    fn push(&mut self, command: Command) {
        if let Some(out) = self.0.as_mut() {
            out.push(command);
        }
    }
}

impl Tree {
    pub fn canvas_geometry_epoch(&self) -> u64 {
        self.canvas_geometry_epoch
    }

    pub(super) fn canvas_geometry_changed(&mut self) {
        self.canvas_geometry_epoch = self.canvas_geometry_epoch.wrapping_add(1);
    }

    /// True when the content box cannot contribute pixels in the surface or
    /// a paint clip. Rounded clips use their enclosing rectangle.
    pub fn canvas_is_offscreen(&self, id: NodeId) -> bool {
        let Some(node) = self.get(id) else {
            return true;
        };
        let mut path = Vec::new();
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(node) = self.get(id) else {
                return false;
            };
            path.push((id, node));
            current = node.parent;
        }
        if path.last().map(|(id, _)| *id) != self.root {
            return true;
        }
        // Effects that can sample or extend beyond a content box leave the
        // decision to the player, as do transforms other than translation.
        if path.iter().any(|(_, node)| {
            let transform = node.transform();
            (node.paints_transform()
                && (transform.rotate_rad != 0.0
                    || transform.scale_x != 1.0
                    || transform.scale_y != 1.0))
                || node.blur() > 0.0
                || node.backdrop() > 0.0
                || node.blend() != crate::scene::BlendMode::SrcOver
        }) {
            return false;
        }

        let (mut x, mut y) = self.origin;
        let mut clips = Vec::new();
        if self.window.0 > 0.0 && self.window.1 > 0.0 {
            clips.push(PaintClip {
                x: 0.0,
                y: 0.0,
                w: self.window.0,
                h: self.window.1,
            });
        } else if let Some((w, h)) = self.viewport {
            clips.push(PaintClip { x, y, w, h });
        }
        for (_, ancestor) in path.into_iter().rev() {
            let effect = paint_kind(ancestor);
            x += ancestor.x;
            y += ancestor.y;
            if effect.transform {
                let transform = ancestor.transform();
                x += transform.translate_x;
                y += transform.translate_y;
            }
            if std::ptr::eq(ancestor, node) {
                break;
            }
            if effect.clip || effect.scroll {
                clips.push(PaintClip {
                    x,
                    y,
                    w: ancestor.w,
                    h: ancestor.h,
                });
            }
            if effect.scroll {
                let (dx, dy) = ancestor.scroll_paint();
                x -= dx;
                y -= dy;
            }
        }
        // Allow for a fractional clip edge sharing a device pixel.
        let edge = self.pixel_scale().recip().max(1.0);
        clips.iter().any(|clip| {
            outside(
                Some(*clip),
                x + node.content_x - edge,
                y + node.content_y - edge,
                node.content_w + 2.0 * edge,
                node.content_h + 2.0 * edge,
            )
        })
    }

    /// Records a flattened command view for inspection and comparison in tests.
    pub fn record(&mut self) -> Result<DrawList, Error> {
        let mut commands = Vec::new();
        self.record_scene(FlatList(Some(&mut commands)))?;
        Ok(DrawList { commands })
    }

    /// Records the compositing layers and their pictures for the current frame.
    pub fn record_layers(&mut self) -> Result<(), Error> {
        self.record_scene(FlatList(None))
    }

    fn record_scene(&mut self, mut out: FlatList<'_>) -> Result<(), Error> {
        self.flush_styles()?;
        if self.needs_layout() {
            return Err(Error::LayoutRequired);
        }
        if let Some(commands) = out.0.as_mut() {
            commands.clear();
        }
        self.sync_scroll_offsets();
        self.sync_metrics();
        self.finish_measure();
        if out.0.is_none() && !self.overlay_visible() && self.apply_layer_patches() {
            self.finish_repaints();
            return Ok(());
        }
        let mut previous = std::mem::take(&mut self.retained_layers.spare);
        std::mem::swap(&mut previous.layers, &mut self.layers.layers);
        std::mem::swap(&mut previous.pictures, &mut self.layers.pictures);
        std::mem::swap(&mut previous.keys, &mut self.retained_layers.keys);
        self.retained_layers.begin();
        self.picture_writes = 0;
        self.layer_record_visits = 0;
        self.layers.layers.clear();
        self.layers.pictures.clear();
        self.retained_layers.keys.clear();
        self.layers.shader_uniforms.clear();
        self.layer_stack.clear();
        self.layer_tail.clear();
        self.layer_top = None;
        self.finished_owners.clear();
        self.begin_compositor_record();
        let root = self.root.ok_or(Error::NoRoot)?;
        let _ = self.sweep_subtree_layers(root);
        self.run = RunState {
            scope: Some(RunScope {
                owner: root,
                base: self.origin,
                mirror: self.origin,
                segment: 0,
            }),
            commands: Vec::new(),
            first: None,
        };
        self.emit_node(root, &mut out, self.origin, (0.0, 0.0), None);
        self.close_run();
        self.leave_owner(root);
        self.finish_compositor_record();
        if self.debug.repaint.is_some() {
            let prior: std::collections::HashMap<_, _> = previous
                .keys
                .iter()
                .zip(&previous.layers)
                .map(|(key, layer)| (*key, &layer.kind))
                .collect();
            for index in 0..self.layers.layers.len() {
                let key = self.retained_layers.keys[index];
                let LayerKey::Node(node, _) = key else {
                    continue;
                };
                if prior
                    .get(&key)
                    .is_none_or(|kind| **kind != self.layers.layers[index].kind)
                {
                    self.commit_repaint(node, super::PaintPath::FullPictureRebuild, None, 0);
                }
            }
        }
        self.finish_repaints();
        if self.overlay_visible() {
            let commands = self.overlay_commands();
            if !commands.is_empty() {
                self.adopt_picture(Arc::new(crate::scene::Picture::new(
                    super::repaint::OVERLAY_PICTURE_ID,
                    self.layer_epoch,
                    (0.0, 0.0),
                    commands.into(),
                )));
            }
        }
        self.runs.retain(|id, _| self.finished_owners.contains(id));
        debug_assert!(self.layer_stack.is_empty());
        if self.layers.epoch != self.layer_epoch {
            self.retained_layers.delta = previous.changes(&self.layers, &self.retained_layers.keys);
        }
        previous.layers.clear();
        previous.pictures.clear();
        previous.keys.clear();
        self.retained_layers.spare = previous;
        self.layers.epoch = self.layer_epoch;
        self.retained_layers.invalid = false;
        Ok(())
    }

    /// Selection fill and composing underline, in paragraph space.
    fn editor_mark_rects(&mut self, id: NodeId) -> Vec<(super::Rect, [u8; 4])> {
        let snapshot = self.get(id).and_then(|node| {
            let text = node.text()?;
            let editor = text.editor.as_ref()?;
            if editor.value().text != text.utf8.as_ref() {
                return None;
            }
            Some((
                editor.value().selection.ordered(),
                editor.value().composing.clone(),
                text.color,
            ))
        });
        let Some(((start, end), composing, color)) = snapshot else {
            return Vec::new();
        };
        let mut marks = Vec::new();
        if start != end
            && let Ok(rects) = self.selection_rects(id, start, end)
        {
            for rect in rects {
                marks.push((rect, [128, 176, 255, 80]));
            }
        }
        if let Some(range) = composing
            && range.end > range.start
            && let Ok(rects) = self.selection_rects(id, range.start, range.end)
        {
            for rect in rects {
                marks.push((
                    super::Rect {
                        x: rect.x,
                        y: (rect.y + rect.h - 2.0).max(rect.y),
                        w: rect.w,
                        h: 2.0_f32.min(rect.h),
                    },
                    color,
                ));
            }
        }
        marks
    }

    /// Commands written into pictures by the last [`Tree::record`]. A replay is zero.
    pub fn picture_writes(&self) -> usize {
        self.picture_writes
    }

    /// Epoch, layer count, and cached picture count from the last record.
    pub fn layer_tree_stats(&self) -> (u64, usize, usize) {
        (
            self.layer_epoch,
            self.layers.layers.len(),
            self.pictures.len() + self.runs.values().map(Vec::len).sum::<usize>(),
        )
    }

    /// Entries visited while assembling layers in the last record.
    pub fn layer_record_visits(&self) -> u32 {
        self.layer_record_visits
    }

    #[cfg(test)]
    pub(crate) fn picture_cache_len(&self) -> usize {
        self.pictures.len() + self.runs.values().map(Vec::len).sum::<usize>()
    }

    /// Nodes `record` walks, including rows whose clipped children are skipped.
    pub fn record_visit_count(&mut self) -> Result<usize, Error> {
        let mut commands = Vec::new();
        Ok(self.record_node(
            self.root.ok_or(Error::NoRoot)?,
            &mut commands,
            RecordAt {
                parent: self.origin,
                offset: (0.0, 0.0),
                space: (0.0, 0.0),
                clip: None,
                descend: true,
            },
        ))
    }

    /// Independent scale for a transform node. Non-finite values or values whose
    /// absolute value is below `1e-4` are rejected and leave the node unchanged.
    pub fn set_scale_xy(&mut self, id: NodeId, scale_x: f32, scale_y: f32) -> Result<(), Error> {
        if !scale_x.is_finite()
            || !scale_y.is_finite()
            || scale_x.abs() < 1.0e-4
            || scale_y.abs() < 1.0e-4
        {
            return Err(Error::BadNumber);
        }
        let was_identity = self.node(id)?.transform().is_identity();
        {
            let node = self.node_mut(id)?;
            let Some(geom) = node.transform_mut() else {
                return Err(Error::WrongKind);
            };
            geom.scale_x = scale_x;
            geom.scale_y = scale_y;
        }
        let is_identity = self.node(id)?.transform().is_identity();
        if was_identity == is_identity {
            if !self.update_transform_slot(id) {
                self.paint_layer_value();
            }
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Center stroke on this node's fill rectangle. Non-positive width clears it.
    pub fn set_stroke(&mut self, id: NodeId, width: f32, color: [u8; 4]) -> Result<(), Error> {
        {
            let node = self.node_mut(id)?;
            node.write_paint(|paint| {
                paint.stroke_width = width;
                paint.stroke_color = color;
            });
        }
        if !self.update_rect_slot(id) {
            self.mark_repaint(id);
        }
        Ok(())
    }

    /// Unit-square gradient. Fewer than two legal stops records as a solid fill.
    pub fn set_gradient(
        &mut self,
        id: NodeId,
        gradient: Option<crate::scene::Gradient>,
    ) -> Result<(), Error> {
        self.node_mut(id)?
            .write_paint(|paint| paint.gradient = gradient);
        self.mark_repaint(id);
        Ok(())
    }

    /// Material shadow. Non-positive elevation records no shadow.
    pub fn set_shadow(
        &mut self,
        id: NodeId,
        shadow: Option<crate::scene::Shadow>,
    ) -> Result<(), Error> {
        self.node_mut(id)?
            .write_paint(|paint| paint.shadow = shadow);
        self.mark_repaint(id);
        Ok(())
    }

    /// Box shadows drawn before the rectangle, in the same picture.
    pub fn set_box_shadows(
        &mut self,
        id: NodeId,
        shadows: impl Into<Arc<[crate::scene::BoxShadow]>>,
    ) -> Result<(), Error> {
        let shadows = shadows.into();
        self.node_mut(id)?
            .write_paint(|paint| paint.box_shadows = shadows);
        self.mark_repaint(id);
        Ok(())
    }

    /// Foreground blur in scene points. Non-positive or non-finite sigma records nothing.
    pub fn set_blur(&mut self, id: NodeId, sigma: f32) -> Result<(), Error> {
        let previous = positive_sigma(self.node(id)?.blur()).is_some();
        self.node_mut(id)?.write_paint(|paint| paint.blur = sigma);
        self.stage_repaint(id, super::RepaintReason::EFFECT_LAYER);
        let next = positive_sigma(self.node(id)?.blur()).is_some();
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Backdrop blur in scene points. Non-positive or non-finite sigma records nothing.
    pub fn set_backdrop_blur(&mut self, id: NodeId, sigma: f32) -> Result<(), Error> {
        let previous = positive_sigma(self.node(id)?.backdrop()).is_some();
        self.node_mut(id)?
            .write_paint(|paint| paint.backdrop = sigma);
        self.stage_repaint(id, super::RepaintReason::EFFECT_LAYER);
        let next = positive_sigma(self.node(id)?.backdrop()).is_some();
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Composites the backdrop with a material inside this node's bounds.
    pub fn set_backdrop_material(
        &mut self,
        id: NodeId,
        material: Option<Arc<crate::scene::BackdropMaterial>>,
    ) -> Result<(), Error> {
        if material
            .as_ref()
            .is_some_and(|material| !material.is_valid())
        {
            return Err(Error::BadNumber);
        }
        let old = self.node(id)?.backdrop_material();
        if old == material.as_ref() {
            return Ok(());
        }
        let same_structure = old.is_some() == material.is_some();
        self.node_mut(id)?
            .write_paint(|paint| paint.backdrop_material = material);
        self.stage_repaint(id, super::RepaintReason::EFFECT_LAYER);
        if same_structure {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Filters this node and its descendants after they have been composited.
    pub fn set_color_filter(
        &mut self,
        id: NodeId,
        matrix: Option<Arc<crate::scene::ColorMatrix>>,
    ) -> Result<(), Error> {
        if matrix.as_ref().is_some_and(|matrix| {
            matrix
                .iter()
                .flatten()
                .any(|coefficient| !coefficient.is_finite())
        }) {
            return Err(Error::BadNumber);
        }
        let old = self.node(id)?.color_filter();
        if old == matrix.as_ref() {
            return Ok(());
        }
        let same_structure = old.is_some() == matrix.is_some();
        self.node_mut(id)?
            .write_paint(|paint| paint.color_filter = matrix);
        self.stage_repaint(id, super::RepaintReason::EFFECT_LAYER);
        if same_structure {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Layer blend. [`crate::scene::BlendMode::SrcOver`] records no push and no pop.
    pub fn set_blend(&mut self, id: NodeId, mode: crate::scene::BlendMode) -> Result<(), Error> {
        let previous = self.node(id)?.blend() != crate::scene::BlendMode::SrcOver;
        self.node_mut(id)?.write_paint(|paint| paint.blend = mode);
        self.stage_repaint(id, super::RepaintReason::EFFECT_LAYER);
        let next = self.node(id)?.blend() != crate::scene::BlendMode::SrcOver;
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    pub(super) fn sweep_subtree_layers(&mut self, id: NodeId) -> bool {
        self.sweep_stack.clear();
        self.sweep_stack.push(SweepFrame {
            id,
            expanded: false,
        });
        while let Some(frame) = self.sweep_stack.pop() {
            if !frame.expanded {
                if self.subtree_layers.contains_key(&frame.id) {
                    continue;
                }
                self.sweep_stack.push(SweepFrame {
                    id: frame.id,
                    expanded: true,
                });
                self.sweep_children.clear();
                let mut child = self.get(frame.id).and_then(|node| node.first_child);
                while let Some(current) = child {
                    self.sweep_children.push(current);
                    child = self.get(current).and_then(|node| node.next);
                }
                for child in self.sweep_children.iter().rev() {
                    self.sweep_stack.push(SweepFrame {
                        id: *child,
                        expanded: false,
                    });
                }
                continue;
            }
            let mut any = false;
            let mut owners = false;
            let mut child = self.get(frame.id).and_then(|node| node.first_child);
            while let Some(current) = child {
                let stored = self
                    .subtree_layers
                    .get(&current)
                    .copied()
                    .unwrap_or_default();
                let renders = self
                    .get(current)
                    .is_some_and(|node| node.renders_layers(stored.requires_clip_layer()));
                any |= stored.effects
                    || renders
                    || self.scroll_bindings.contains_key(&current)
                    || self.uses_paint_slot(current);
                owners |= stored.picture_owners || self.is_picture_owner(current);
                child = self.get(current).and_then(|node| node.next);
            }
            self.subtree_layers.insert(
                frame.id,
                SubtreeLayers {
                    effects: any,
                    picture_owners: owners,
                },
            );
        }
        self.renders_layer_node(id) || !self.subtree_flat(id)
    }

    pub(super) fn invalidate_subtree_layers(&mut self, id: NodeId) {
        let mut current = Some(id);
        while let Some(id) = current {
            if self.subtree_layers.remove(&id).is_none() {
                break;
            }
            current = self.get(id).and_then(|node| node.parent);
        }
    }

    fn subtree_flat(&self, id: NodeId) -> bool {
        !self
            .subtree_layers
            .get(&id)
            .is_some_and(|value| value.effects)
    }

    fn picture_fresh(&self, id: NodeId) -> bool {
        self.pictures.contains_key(&id) && !self.is_repaint_dirty(id)
    }

    fn capture(
        &mut self,
        id: NodeId,
        parent_abs: (f32, f32),
        out: &mut FlatList<'_>,
        clip: Option<PaintClip>,
    ) {
        let space = self.run.scope.map(|scope| scope.base).unwrap_or((0.0, 0.0));
        let mut buf = std::mem::take(&mut self.run.commands);
        let before = buf.len();
        let _ = self.record_node(
            id,
            &mut buf,
            RecordAt {
                parent: parent_abs,
                offset: (0.0, 0.0),
                space,
                clip: None,
                descend: false,
            },
        );
        if self.run.first.is_none() && buf.len() > before {
            self.run.first = Some(id);
        }
        if buf.len() > before
            && let Some(tracker) = self.debug.repaint.as_mut()
        {
            tracker.run_members.push(id);
        }
        self.mirror_tail(before, &buf, out, clip);
        self.run.commands = buf;
    }

    fn mirror_tail(
        &self,
        before: usize,
        buf: &[Command],
        out: &mut FlatList<'_>,
        clip: Option<PaintClip>,
    ) {
        let (Some(scope), Some(out)) = (self.run.scope.as_ref(), out.0.as_deref_mut()) else {
            return;
        };
        self.replay_commands(&buf[before..], scope.mirror.0, scope.mirror.1, clip, out);
    }

    fn local_xy(&self, abs: (f32, f32)) -> (f32, f32) {
        let base = self.run.scope.map(|scope| scope.base).unwrap_or((0.0, 0.0));
        (abs.0 - base.0, abs.1 - base.1)
    }

    fn push_run_bracket(
        &mut self,
        source: Option<NodeId>,
        command: Command,
        out: &mut FlatList<'_>,
    ) {
        if self.run.first.is_none()
            && let Some(source) = source
        {
            self.run.first = Some(source);
        }
        if let Some(out) = out.0.as_mut() {
            let mirror = self.run.scope.map_or((0.0, 0.0), |scope| scope.mirror);
            out.push(translate_command(&command, mirror.0, mirror.1));
        }
        self.run.commands.push(command);
    }

    fn close_run(&mut self) {
        let Some(scope) = self.run.scope else {
            return;
        };
        let members = self
            .debug
            .repaint
            .as_mut()
            .map(|tracker| std::mem::take(&mut tracker.run_members));
        let captured = std::mem::take(&mut self.run.commands);
        let first = self.run.first.take();
        if captured.is_empty() {
            return;
        }
        let Some(first) = first else {
            debug_assert!(false, "run with commands has a first node");
            return;
        };
        let index = scope.segment;
        if let Some(slot) = self.run.scope.as_mut() {
            slot.segment = index + 1;
        }
        let picture_id = crate::tree::picture_id::self_picture_id(first);
        let dirty = self.is_repaint_dirty(scope.owner);
        if !dirty
            && let Some(matched) = self.runs.get(&scope.owner).and_then(|runs| runs.get(index))
            && matched.id == picture_id
            && matched.commands().as_ref() == captured.as_slice()
        {
            self.adopt_picture(Arc::clone(matched));
            if let (Some(tracker), Some(mut members)) = (self.debug.repaint.as_mut(), members) {
                members.clear();
                tracker.run_members = members;
            }
            return;
        }
        if let Some(mut members) = members {
            self.commit_repaint(
                scope.owner,
                super::PaintPath::FullPictureRebuild,
                Some(picture_id),
                captured.len(),
            );
            for node in members.drain(..) {
                self.commit_repaint(
                    node,
                    super::PaintPath::FullPictureRebuild,
                    Some(picture_id),
                    captured.len(),
                );
            }
            if let Some(tracker) = self.debug.repaint.as_mut() {
                tracker.run_members = members;
            }
        }
        self.picture_writes = self.picture_writes.saturating_add(captured.len());
        let picture = Arc::new(crate::scene::Picture::new(
            picture_id,
            self.layer_epoch,
            scope.base,
            Arc::from(captured),
        ));
        self.store_run(scope.owner, index, Arc::clone(&picture));
        self.adopt_picture(picture);
    }

    fn store_run(&mut self, owner: NodeId, index: usize, picture: Arc<crate::scene::Picture>) {
        let runs = self.runs.entry(owner).or_default();
        if index == 0 {
            runs.clear();
        } else {
            runs.truncate(index);
        }
        runs.push(picture);
    }

    fn leave_owner(&mut self, owner: NodeId) {
        let end = self.run.scope.map(|scope| scope.segment).unwrap_or(0);
        if end == 0 {
            self.runs.remove(&owner);
        } else if let Some(runs) = self.runs.get_mut(&owner) {
            runs.truncate(end);
            self.finished_owners.push(owner);
        }
        self.clear_repaint(owner);
    }

    fn fill_children(&mut self, id: NodeId, abs: (f32, f32)) {
        let mut commands = Vec::new();
        let mut child = self.get(id).and_then(|node| node.first_child);
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            let _ = self.record_node(
                current,
                &mut commands,
                RecordAt {
                    parent: abs,
                    offset: (0.0, 0.0),
                    space: abs,
                    clip: None,
                    descend: true,
                },
            );
            child = next;
        }
        #[cfg(debug_assertions)]
        {
            let mut again = Vec::new();
            let mut child = self.get(id).and_then(|node| node.first_child);
            while let Some(current) = child {
                let next = self.get(current).and_then(|node| node.next);
                let _ = self.record_node(
                    current,
                    &mut again,
                    RecordAt {
                        parent: abs,
                        offset: (0.0, 0.0),
                        space: abs,
                        clip: None,
                        descend: true,
                    },
                );
                child = next;
            }
            debug_assert_eq!(commands, again);
        }
        self.store_picture(id, commands, abs);
    }

    fn emit_picture_owner(
        &mut self,
        id: NodeId,
        out: &mut FlatList<'_>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        let Some((x, y)) = self.get(id).map(|node| (node.x, node.y)) else {
            return;
        };
        let abs = (parent_abs.0 + x, parent_abs.1 + y);
        // The parent's commands before this owner must stay below its picture.
        self.close_run();
        // Scroll clips in scene space and only translates by the scroll offset.
        // `local_xy` is relative to that scroll, so using it parks the row above
        // the clip by whatever chrome sits over the list.
        self.push_layer(id, crate::scene::LayerKind::Offset { x: abs.0, y: abs.1 });
        let picture_index = self.layers.pictures.len();
        let layer_index = self.layers.layers.len();
        if !self.picture_fresh(id) {
            self.fill_owner(id, abs);
        } else if let Some(picture) = self.pictures.get(&id).cloned() {
            self.adopt_picture(picture);
        }
        if self.layers.pictures.len() > picture_index {
            self.retained_layers.owners.insert(
                id,
                RecordedOwner {
                    picture: picture_index,
                    layer: crate::scene::LayerId(crate::cast::u32_from_usize(layer_index)),
                    origin: abs,
                },
            );
        }
        let dx = abs.0 - ancestor_scroll.0;
        let dy = abs.1 - ancestor_scroll.1;
        self.blit(id, out, dx, dy, clip);
        self.pop_layers(1);
    }

    /// Row-local commands. The caller's `Offset` layer places the picture.
    fn fill_owner(&mut self, id: NodeId, abs: (f32, f32)) {
        let commands = self.record_owner(id, abs);
        // The offset layer places these commands in the scene.
        self.store_picture(id, commands, (0.0, 0.0));
    }

    fn record_owner(&mut self, id: NodeId, abs: (f32, f32)) -> Vec<Command> {
        let Some((x, y)) = self.get(id).map(|node| (node.x, node.y)) else {
            return Vec::new();
        };
        let parent = (abs.0 - x, abs.1 - y);
        let mut commands = Vec::new();
        let visits = self.record_node(
            id,
            &mut commands,
            RecordAt {
                parent,
                offset: (0.0, 0.0),
                space: abs,
                clip: None,
                descend: true,
            },
        );
        self.layer_record_visits = self
            .layer_record_visits
            .saturating_add(crate::cast::u32_from_usize(visits));
        commands
    }

    fn store_picture(&mut self, id: NodeId, commands: Vec<Command>, origin: (f32, f32)) {
        if let Some(previous) = self.pictures.get(&id)
            && previous.origin == origin
            && previous.commands().as_ref() == commands.as_slice()
        {
            self.adopt_picture(Arc::clone(previous));
            self.clear_repaint(id);
            return;
        }
        self.picture_writes = self.picture_writes.saturating_add(commands.len());
        let picture_id = crate::tree::picture_id::child_picture_id(id);
        self.commit_picture_repaint(
            id,
            super::PaintPath::FullPictureRebuild,
            picture_id,
            commands.len(),
        );
        let picture = Arc::new(crate::scene::Picture::new(
            picture_id,
            self.layer_epoch,
            origin,
            Arc::from(commands),
        ));
        self.pictures.insert(id, Arc::clone(&picture));
        self.adopt_picture(picture);
        self.clear_repaint(id);
    }

    fn adopt_picture(&mut self, picture: Arc<crate::scene::Picture>) {
        let id = picture.id;
        self.layers.pictures.push(picture);
        self.append_layer(
            LayerKey::Picture(id),
            crate::scene::LayerKind::Picture { picture: id },
        );
    }

    fn emit_platform_view(&mut self, id: NodeId) {
        if self.records_platform_view(id) {
            self.close_run();
            self.note_layer(
                id,
                crate::scene::LayerKind::PlatformView { node: id.to_raw() },
            );
        }
    }

    fn clear_repaint(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags.remove(super::node::NodeFlags::REPAINT_DIRTY);
        }
    }

    fn note_layer(&mut self, node: NodeId, kind: crate::scene::LayerKind) -> crate::scene::LayerId {
        self.append_layer(LayerKey::Node(node, std::mem::discriminant(&kind)), kind)
    }

    fn append_layer(
        &mut self,
        key: LayerKey,
        kind: crate::scene::LayerKind,
    ) -> crate::scene::LayerId {
        self.retained_layers.keys.push(key);
        let id = crate::scene::LayerId(crate::cast::u32_from_usize(self.layers.layers.len()));
        let parent = self.layer_stack.last().copied();
        self.layers.layers.push(crate::scene::Layer {
            kind,
            parent,
            first_child: None,
            next: None,
        });
        self.layer_tail.push(None);
        if let Some(parent) = parent {
            self.attach_layer_child(parent, id);
        } else if let Some(tail) = self.layer_top {
            self.layers.layers[tail.0 as usize].next = Some(id);
            self.layer_top = Some(id);
        } else {
            self.layers.root = id;
            self.layer_top = Some(id);
        }
        id
    }

    fn attach_layer_child(&mut self, parent: crate::scene::LayerId, child: crate::scene::LayerId) {
        let slot = parent.0 as usize;
        if let Some(tail) = self.layer_tail[slot] {
            self.layers.layers[tail.0 as usize].next = Some(child);
        } else {
            self.layers.layers[slot].first_child = Some(child);
        }
        self.layer_tail[slot] = Some(child);
    }

    fn push_layer(&mut self, node: NodeId, kind: crate::scene::LayerKind) -> crate::scene::LayerId {
        let id = self.note_layer(node, kind);
        self.layer_stack.push(id);
        id
    }

    fn pop_layers(&mut self, count: usize) {
        for _ in 0..count {
            self.layer_stack.pop();
        }
    }
}

/// Effects that can sit on one node together.
#[derive(Clone, Copy)]
struct PaintKind {
    scroll: bool,
    clip: bool,
    opacity: bool,
    transform: bool,
}

fn paint_kind(node: &Node) -> PaintKind {
    PaintKind {
        scroll: node.is_scroll(),
        clip: node.role.contains(Role::CLIP),
        opacity: node.pushes_opacity(),
        transform: node.paints_transform(),
    }
}

pub(super) fn positive_sigma(sigma: f32) -> Option<f32> {
    if sigma.is_finite() && sigma > 0.0 {
        Some(sigma)
    } else {
        None
    }
}

pub(super) struct RectPaint {
    fill: Option<[u8; 4]>,
    stroke_width: f32,
    stroke_color: [u8; 4],
    gradient: Option<crate::scene::Gradient>,
    shadow: Option<crate::scene::Shadow>,
    box_shadows: Arc<[crate::scene::BoxShadow]>,
}

pub(super) fn rect_paint(node: &Node) -> RectPaint {
    let stroke_width = if node.stroke_width().is_finite() && node.stroke_width() > 0.0 {
        node.stroke_width()
    } else {
        0.0
    };
    let shadow = node
        .shadow()
        .filter(|shadow| shadow.elevation.is_finite() && shadow.elevation > 0.0);
    RectPaint {
        fill: node.fill(),
        stroke_width,
        stroke_color: node.stroke_color(),
        gradient: node.gradient().cloned(),
        shadow,
        box_shadows: node.box_shadows(),
    }
}

pub(super) enum PreparedGradient {
    Paint(crate::scene::Gradient),
    Solid([u8; 4]),
    Drop,
}

pub(super) fn prepared_gradient(gradient: &crate::scene::Gradient) -> PreparedGradient {
    let stops = match gradient {
        crate::scene::Gradient::Linear { stops, .. }
        | crate::scene::Gradient::Radial { stops, .. }
        | crate::scene::Gradient::Sweep { stops, .. } => stops,
    };
    if stops.len() < 2
        || stops
            .iter()
            .any(|stop| !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset))
    {
        return PreparedGradient::Drop;
    }
    let mut stops = stops.clone();
    stops.sort_by(|left, right| {
        left.offset
            .partial_cmp(&right.offset)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    match gradient {
        crate::scene::Gradient::Linear { start, end, .. } => {
            if !start
                .iter()
                .chain(end.iter())
                .all(|value| value.is_finite())
            {
                return PreparedGradient::Drop;
            }
            if start == end {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0, 0, 0, 0]),
                );
            }
            PreparedGradient::Paint(crate::scene::Gradient::Linear {
                start: *start,
                end: *end,
                stops,
            })
        }
        crate::scene::Gradient::Radial { center, radius, .. } => {
            if !center[0].is_finite()
                || !center[1].is_finite()
                || !radius.is_finite()
                || *radius <= 0.0
            {
                return PreparedGradient::Drop;
            }
            PreparedGradient::Paint(crate::scene::Gradient::Radial {
                center: *center,
                radius: *radius,
                stops,
            })
        }
        crate::scene::Gradient::Sweep {
            center,
            start_angle_rad,
            end_angle_rad,
            ..
        } => {
            if !center[0].is_finite()
                || !center[1].is_finite()
                || !start_angle_rad.is_finite()
                || !end_angle_rad.is_finite()
            {
                return PreparedGradient::Drop;
            }
            if start_angle_rad == end_angle_rad {
                return PreparedGradient::Solid(
                    stops.last().map(|stop| stop.color).unwrap_or([0, 0, 0, 0]),
                );
            }
            PreparedGradient::Paint(crate::scene::Gradient::Sweep {
                center: *center,
                start_angle_rad: *start_angle_rad,
                end_angle_rad: *end_angle_rad,
                stops,
            })
        }
    }
}

fn outside(clip: Option<PaintClip>, x: f32, y: f32, w: f32, h: f32) -> bool {
    let Some(clip) = clip else {
        return false;
    };
    x + w <= clip.x || y + h <= clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h
}

fn intersect(clip: Option<PaintClip>, x: f32, y: f32, w: f32, h: f32) -> Option<PaintClip> {
    let Some(clip) = clip else {
        return Some(PaintClip { x, y, w, h });
    };
    let left = clip.x.max(x);
    let top = clip.y.max(y);
    let right = (clip.x + clip.w).min(x + w);
    let bottom = (clip.y + clip.h).min(y + h);
    if right <= left || bottom <= top {
        None
    } else {
        Some(PaintClip {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        })
    }
}

#[cfg(test)]
mod record_bench {
    use super::*;
    use crate::text::MonoShaper;
    use std::hint::black_box;
    use std::time::{Duration, Instant};

    const WARMUP: usize = 10;
    const SAMPLES: usize = 40;
    const P95_INDEX: usize = 37;

    fn build_rows(n: usize, scrolled: bool) -> Result<(Tree, Option<NodeId>, NodeId), Error> {
        let mut tree = Tree::new(MonoShaper::default());
        let scroll = if scrolled { Some(tree.scroll()?) } else { None };
        let column = tree.column(0.0)?;
        for index in 0..n {
            let row = tree.pointer_region()?;
            let pad = tree.padding(4.0)?;
            let deco = tree.decorated([20, 20, 20, 255], 0.0)?;
            let text = tree.text(format!("Row {index}"), 16.0, [240, 240, 240, 255])?;
            tree.add_child(deco, text)?;
            tree.add_child(pad, deco)?;
            tree.add_child(row, pad)?;
            tree.add_child(column, row)?;
        }
        if let Some(scroll) = scroll {
            tree.add_child(scroll, column)?;
        }
        Ok((tree, scroll, column))
    }

    fn p95(samples: &mut [Duration]) -> Duration {
        samples.sort();
        samples[P95_INDEX]
    }

    fn sample_pass(tree: &mut Tree, previous: &DrawList) -> (Duration, Duration, Duration) {
        let started = Instant::now();
        tree.layout().unwrap();
        let layout = started.elapsed();
        let started = Instant::now();
        let list = tree.record().unwrap();
        let record = started.elapsed();
        let started = Instant::now();
        let _ = black_box(previous == &list);
        let compare = started.elapsed();
        black_box(list);
        (layout, record, compare)
    }

    fn run_samples(n: usize, viewport_h: f32) -> Result<(Duration, Duration, Duration), Error> {
        let (mut tree, _, _) = build_rows(n, false)?;
        tree.set_viewport(0.0, 0.0, 360.0, viewport_h)?;
        tree.layout()?;
        let mut previous = tree.record()?;
        let mut layouts = Vec::with_capacity(SAMPLES);
        let mut records = Vec::with_capacity(SAMPLES);
        let mut compares = Vec::with_capacity(SAMPLES);
        for index in 0..WARMUP + SAMPLES {
            let (layout, record, compare) = sample_pass(&mut tree, &previous);
            previous = tree.record()?;
            if index >= WARMUP {
                layouts.push(layout);
                records.push(record);
                compares.push(compare);
            }
        }
        Ok((p95(&mut layouts), p95(&mut records), p95(&mut compares)))
    }

    #[test]
    #[ignore]
    fn record_bench() {
        for n in [100usize, 1000, 5000] {
            let height = 1_000_000.0;
            let (layout, record, compare) = run_samples(n, height).expect("bench tree");
            println!(
                "visible n={n} layout_p95={:.3}ms record_p95={:.3}ms compare_p95={:.3}ms",
                layout.as_secs_f64() * 1000.0,
                record.as_secs_f64() * 1000.0,
                compare.as_secs_f64() * 1000.0
            );
            if n == 5000 && cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                assert!(
                    record.as_secs_f64() * 1000.0 <= 2.0,
                    "record p95 {}ms",
                    record.as_secs_f64() * 1000.0
                );
                assert!(
                    compare.as_secs_f64() * 1000.0 <= 1.0,
                    "compare p95 {}ms",
                    compare.as_secs_f64() * 1000.0
                );
            }
        }

        let (mut tree, scroll, column) = build_rows(5000, true).expect("scroll tree");
        let scroll = scroll.expect("scroll");
        tree.set_viewport(0.0, 0.0, 360.0, 640.0).expect("viewport");
        tree.layout().expect("layout");
        let content = tree.rect(column).expect("content");
        tree.set_scroll_offset(scroll, 0.0, (content.h * 0.5 - 320.0).max(0.0))
            .expect("offset");
        let started = Instant::now();
        let visits = tree.record_visit_count().expect("visits");
        let record = started.elapsed();
        println!(
            "scroll 5000 visits={visits} record={:.3}ms",
            record.as_secs_f64() * 1000.0
        );
    }
}
