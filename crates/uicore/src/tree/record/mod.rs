//! Record a [`DrawList`] from the laid-out tree.

use std::sync::Arc;

use protocol::{Command, DrawList};

use super::node::Kind;
use super::{NodeId, Tree};
use crate::Error;

mod pass;
use pass::{RecordFrame, emit_image, emit_rect, emit_text};

/// One node in the iterative layer sweep. `expanded` means its children are already stacked.
#[derive(Clone, Copy)]
pub(super) struct SweepFrame {
    id: NodeId,
    expanded: bool,
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

/// In-progress run. `scope` is `None` outside [`Tree::record_into`].
#[derive(Default)]
pub(super) struct RunState {
    scope: Option<RunScope>,
    commands: Vec<Command>,
    first: Option<NodeId>,
}

impl Tree {
    pub fn record(&mut self) -> Result<DrawList, Error> {
        let mut commands = Vec::new();
        self.record_into(&mut commands)?;
        Ok(DrawList { commands })
    }

    /// Clears `commands` and refills them. The caller can keep the buffer's capacity.
    ///
    /// A run is the commands between layer boundaries, owned by the root or by the
    /// node that opens those layers. A clean owner adopts its cached runs. Scroll
    /// still keeps one child picture.
    pub fn record_into(&mut self, commands: &mut Vec<Command>) -> Result<(), Error> {
        commands.clear();
        self.sync_paragraphs();
        self.picture_writes = 0;
        self.layers.layers.clear();
        self.layers.pictures.clear();
        self.layer_stack.clear();
        self.layer_tail.clear();
        self.layer_top = None;
        self.finished_owners.clear();
        self.subtree_layers.clear();
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
        self.emit_node(root, commands, self.origin, (0.0, 0.0), None);
        self.close_run();
        self.leave_owner(root);
        self.runs.retain(|id, _| self.finished_owners.contains(id));
        debug_assert!(self.layer_stack.is_empty());
        self.layers.epoch = self.layer_epoch;
        Ok(())
    }

    /// Selection fill and composing underline, in paragraph space.
    fn editor_mark_rects(&self, id: NodeId) -> Vec<(super::Rect, [u8; 4])> {
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

    #[cfg(test)]
    pub(crate) fn picture_cache_len(&self) -> usize {
        self.pictures.len() + self.runs.values().map(Vec::len).sum::<usize>()
    }

    /// Nodes `record` walks, including rows whose clipped children are skipped.
    pub fn record_visit_count(&self) -> Result<usize, Error> {
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
            self.paint_layer_value();
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
        self.mark_repaint(id);
        Ok(())
    }

    /// Unit-square gradient. Fewer than two legal stops records as a solid fill.
    pub fn set_gradient(
        &mut self,
        id: NodeId,
        gradient: Option<protocol::Gradient>,
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
        shadow: Option<protocol::Shadow>,
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
        shadows: impl Into<Arc<[protocol::BoxShadow]>>,
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
        let next = positive_sigma(self.node(id)?.backdrop()).is_some();
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    /// Layer blend. [`protocol::BlendMode::SrcOver`] records no push and no pop.
    pub fn set_blend(&mut self, id: NodeId, mode: protocol::BlendMode) -> Result<(), Error> {
        let previous = self.node(id)?.blend() != protocol::BlendMode::SrcOver;
        self.node_mut(id)?.write_paint(|paint| paint.blend = mode);
        let next = self.node(id)?.blend() != protocol::BlendMode::SrcOver;
        if previous == next {
            self.paint_layer_value();
        } else {
            self.repaint_ancestors(id);
        }
        Ok(())
    }

    fn record_node(&self, id: NodeId, commands: &mut Vec<Command>, at: RecordAt) -> usize {
        let marks = self.editor_mark_rects(id);
        let Some(node) = self.get(id) else {
            return 0;
        };
        let abs_x = at.parent.0 + node.x;
        let abs_y = at.parent.1 + node.y;
        let x = abs_x - at.offset.0 - at.space.0;
        let y = abs_y - at.offset.1 - at.space.1;
        if node.kind != Kind::Transform && outside(at.clip, x, y, node.w, node.h) {
            return 1;
        }
        emit_rect(commands, node, x, y);
        emit_text(self.text_scale, commands, node, x, y, &marks);
        emit_image(commands, node, x, y);
        if !at.descend {
            return 1;
        }
        // A settled opacity composites nothing: children draw straight into the
        // current picture. A zero alpha skips the subtree entirely.
        if node.kind == Kind::Opacity && node.alpha() == Some(0) {
            return 1;
        }
        let scroll = node.scroll_paint();
        let child_offset = if node.kind == Kind::Scroll {
            (at.offset.0 + scroll.0, at.offset.1 + scroll.1)
        } else {
            at.offset
        };
        let child_clip = match node.kind {
            Kind::Transform => None,
            Kind::Scroll | Kind::Clip => intersect(at.clip, x, y, node.w, node.h),
            _ => at.clip,
        };
        let mut child = node.first_child;
        let frame = RecordFrame::capture(node, x, y);
        frame.push(commands);
        let mut visits = 1;
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            visits += self.record_node(
                current,
                commands,
                RecordAt {
                    parent: (abs_x, abs_y),
                    offset: child_offset,
                    space: at.space,
                    clip: child_clip,
                    descend: true,
                },
            );
            child = next;
        }
        frame.pop(commands);
        visits
    }

    fn emit_node(
        &mut self,
        id: NodeId,
        out: &mut Vec<Command>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        let Some(node) = self.get(id) else {
            return;
        };
        let kind = node.kind;
        let alpha = node.alpha();
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        let mut child = node.first_child;
        if kind == Kind::Opacity && alpha == Some(0) {
            self.capture(id, parent_abs, out, clip);
            return;
        }
        if kind == Kind::Clip && !self.renders_layer_node(id) {
            self.emit_settled_clip(id, out, parent_abs, ancestor_scroll, clip, child);
            return;
        }
        if self.renders_layer_node(id) {
            self.emit_layer(id, out, parent_abs, ancestor_scroll, clip);
            return;
        }
        self.capture(id, parent_abs, out, clip);
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            self.emit_node(current, out, abs, ancestor_scroll, clip);
            child = next;
        }
    }

    fn emit_settled_clip(
        &mut self,
        id: NodeId,
        out: &mut Vec<Command>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
        mut child: Option<NodeId>,
    ) {
        let Some(node) = self.get(id) else {
            return;
        };
        let (w, h) = (node.w, node.h);
        let radius = node.corner_radius();
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        self.capture(id, parent_abs, out, clip);
        let paint = (abs.0 - ancestor_scroll.0, abs.1 - ancestor_scroll.1);
        let local = self.local_xy(abs);
        self.push_run_bracket(
            Some(id),
            Command::PushClip {
                x: local.0,
                y: local.1,
                w,
                h,
                corner_radius: radius,
            },
            out,
        );
        let child_clip = intersect(clip, paint.0, paint.1, w, h);
        while let Some(current) = child {
            let next = self.get(current).and_then(|node| node.next);
            self.emit_node(current, out, abs, ancestor_scroll, child_clip);
            child = next;
        }
        self.push_run_bracket(None, Command::Pop, out);
    }

    fn emit_layer(
        &mut self,
        id: NodeId,
        out: &mut Vec<Command>,
        parent_abs: (f32, f32),
        ancestor_scroll: (f32, f32),
        clip: Option<PaintClip>,
    ) {
        let Some(node) = self.get(id) else {
            return;
        };
        let abs = (parent_abs.0 + node.x, parent_abs.1 + node.y);
        let paint = (abs.0 - ancestor_scroll.0, abs.1 - ancestor_scroll.1);
        let (w, h, kind) = (node.w, node.h, node.kind);
        let radius = node.corner_radius();
        let alpha = node.alpha();
        let scroll = node.scroll_paint();
        let geom = node.transform();
        let transform = (
            geom.translate_x,
            geom.translate_y,
            geom.rotate_rad,
            geom.scale_x,
            geom.scale_y,
        );
        let blur = positive_sigma(node.blur());
        let backdrop = positive_sigma(node.backdrop());
        let blend = node.blend();
        let blend = (blend != protocol::BlendMode::SrcOver).then_some(blend);
        let mut child = node.first_child;
        let caches = node.caches_picture();
        self.capture(id, parent_abs, out, clip);
        let pushes_opacity = kind == Kind::Opacity && alpha.unwrap_or(255) != 255;
        let pushes_matrix = kind == Kind::Transform && !geom.is_identity();
        match kind {
            Kind::Clip => out.push(Command::PushClip {
                x: paint.0,
                y: paint.1,
                w,
                h,
                corner_radius: radius,
            }),
            Kind::Opacity if pushes_opacity => out.push(Command::PushOpacity {
                x: paint.0,
                y: paint.1,
                w,
                h,
                alpha: alpha.unwrap_or(255),
            }),
            Kind::Scroll => out.push(Command::PushClip {
                x: paint.0,
                y: paint.1,
                w,
                h,
                corner_radius: 0.0,
            }),
            Kind::Transform if pushes_matrix => out.push(Command::PushTransform {
                translate_x: transform.0,
                translate_y: transform.1,
                rotate_rad: transform.2,
                scale_x: transform.3,
                scale_y: transform.4,
            }),
            _ => {}
        }
        if let Some(mode) = blend {
            out.push(Command::PushBlend {
                x: paint.0,
                y: paint.1,
                w,
                h,
                mode,
            });
        }
        if let Some(sigma) = blur {
            out.push(Command::PushBlur {
                x: paint.0,
                y: paint.1,
                w,
                h,
                sigma,
            });
        }
        if let Some(sigma) = backdrop {
            out.push(Command::PushBackdropBlur {
                x: paint.0,
                y: paint.1,
                w,
                h,
                sigma,
            });
        }
        self.close_run();
        let Some(saved) = self.run.scope else {
            debug_assert!(false, "record scope");
            return;
        };
        let next_scroll = if kind == Kind::Scroll {
            (ancestor_scroll.0 + scroll.0, ancestor_scroll.1 + scroll.1)
        } else {
            ancestor_scroll
        };
        let child_clip = match kind {
            Kind::Transform => None,
            Kind::Scroll | Kind::Clip => intersect(clip, paint.0, paint.1, w, h),
            _ => clip,
        };
        self.run.scope = Some(RunScope {
            owner: id,
            base: abs,
            mirror: (abs.0 - next_scroll.0, abs.1 - next_scroll.1),
            segment: if saved.owner == id { saved.segment } else { 0 },
        });
        self.run.commands.clear();
        self.run.first = None;
        let pushed = self.push_effect_layers(
            kind, abs, w, h, radius, alpha, scroll, transform, blur, backdrop, blend,
        );
        if caches && (self.picture_fresh(id) || self.subtree_flat(id)) {
            if !self.picture_fresh(id) {
                self.fill_children(id, abs);
            } else if let Some(picture) = self.pictures.get(&id).cloned() {
                self.adopt_picture(picture);
            }
            let dx = abs.0 - next_scroll.0;
            let dy = abs.1 - next_scroll.1;
            self.blit(id, out, dx, dy, child_clip);
        } else {
            self.pictures.remove(&id);
            while let Some(current) = child {
                let next = self.get(current).and_then(|node| node.next);
                self.emit_node(current, out, abs, next_scroll, child_clip);
                child = next;
            }
        }
        self.close_run();
        if saved.owner != id {
            self.leave_owner(id);
            self.run.scope = Some(saved);
        } else if let Some(scope) = self.run.scope.as_mut() {
            scope.base = saved.base;
            scope.mirror = saved.mirror;
        }
        self.run.commands.clear();
        self.run.first = None;
        self.pop_layers(pushed);
        if backdrop.is_some() {
            out.push(Command::Pop);
        }
        if blur.is_some() {
            out.push(Command::Pop);
        }
        if blend.is_some() {
            out.push(Command::Pop);
        }
        if matches!(kind, Kind::Clip | Kind::Scroll) || pushes_opacity || pushes_matrix {
            out.push(Command::Pop);
        }
    }

    pub(super) fn sweep_subtree_layers(&mut self, id: NodeId) -> bool {
        self.sweep_stack.clear();
        self.sweep_stack.push(SweepFrame {
            id,
            expanded: false,
        });
        let mut root_result = false;
        while let Some(frame) = self.sweep_stack.pop() {
            if !frame.expanded {
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
            let mut child = self.get(frame.id).and_then(|node| node.first_child);
            while let Some(current) = child {
                let stored = self.subtree_layers.get(&current).copied().unwrap_or(false);
                let renders = self
                    .get(current)
                    .is_some_and(|node| node.renders_layers(stored));
                any |= stored || renders;
                child = self.get(current).and_then(|node| node.next);
            }
            let renders = self
                .get(frame.id)
                .is_some_and(|node| node.renders_layers(any));
            self.subtree_layers.insert(frame.id, any);
            root_result = any || renders;
        }
        root_result
    }

    fn subtree_flat(&self, id: NodeId) -> bool {
        !self.subtree_layers.get(&id).copied().unwrap_or(false)
    }

    fn picture_fresh(&self, id: NodeId) -> bool {
        self.pictures.contains_key(&id) && !self.is_repaint_dirty(id)
    }

    fn capture(
        &mut self,
        id: NodeId,
        parent_abs: (f32, f32),
        out: &mut Vec<Command>,
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
        self.mirror_tail(before, &buf, out, clip);
        self.run.commands = buf;
    }

    fn mirror_tail(
        &self,
        before: usize,
        buf: &[Command],
        out: &mut Vec<Command>,
        clip: Option<PaintClip>,
    ) {
        let Some(scope) = self.run.scope.as_ref() else {
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
        out: &mut Vec<Command>,
    ) {
        if self.run.first.is_none()
            && let Some(source) = source
        {
            self.run.first = Some(source);
        }
        let mirror = self
            .run
            .scope
            .map(|scope| scope.mirror)
            .unwrap_or((0.0, 0.0));
        let flat = translate_command(&command, mirror.0, mirror.1);
        self.run.commands.push(command);
        out.push(flat);
    }

    fn close_run(&mut self) {
        let Some(scope) = self.run.scope else {
            return;
        };
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
        let picture_id = crate::layer::self_picture_id(first);
        let dirty = self.is_repaint_dirty(scope.owner);
        if !dirty
            && let Some(matched) = self.runs.get(&scope.owner).and_then(|runs| runs.get(index))
            && matched.id == picture_id
            && matched.commands.as_ref() == captured.as_slice()
        {
            self.adopt_picture(Arc::clone(matched));
            return;
        }
        self.picture_writes = self.picture_writes.saturating_add(captured.len());
        let picture = Arc::new(protocol::Picture {
            id: picture_id,
            generation: self.layer_epoch,
            origin: scope.base,
            commands: Arc::from(captured),
        });
        self.store_run(scope.owner, index, Arc::clone(&picture));
        self.adopt_picture(picture);
    }

    fn store_run(&mut self, owner: NodeId, index: usize, picture: Arc<protocol::Picture>) {
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

    fn store_picture(&mut self, id: NodeId, commands: Vec<Command>, origin: (f32, f32)) {
        self.picture_writes = self.picture_writes.saturating_add(commands.len());
        let picture = Arc::new(protocol::Picture {
            id: crate::layer::child_picture_id(id),
            generation: self.layer_epoch,
            origin,
            commands: Arc::from(commands),
        });
        self.pictures.insert(id, Arc::clone(&picture));
        self.adopt_picture(picture);
        self.clear_repaint(id);
    }

    fn adopt_picture(&mut self, picture: Arc<protocol::Picture>) {
        let id = picture.id;
        self.layers.pictures.push(picture);
        self.note_layer(crate::layer::LayerKind::Picture { picture: id });
    }

    fn clear_repaint(&mut self, id: NodeId) {
        if let Ok(node) = self.node_mut(id) {
            node.flags.remove(super::node::NodeFlags::REPAINT_DIRTY);
        }
    }

    fn note_layer(&mut self, kind: crate::layer::LayerKind) -> crate::layer::LayerId {
        let id = crate::layer::LayerId(crate::cast::u32_from_usize(self.layers.layers.len()));
        let parent = self.layer_stack.last().copied();
        self.layers.layers.push(crate::layer::Layer {
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

    fn attach_layer_child(&mut self, parent: crate::layer::LayerId, child: crate::layer::LayerId) {
        let slot = parent.0 as usize;
        if let Some(tail) = self.layer_tail[slot] {
            self.layers.layers[tail.0 as usize].next = Some(child);
        } else {
            self.layers.layers[slot].first_child = Some(child);
        }
        self.layer_tail[slot] = Some(child);
    }

    fn push_layer(&mut self, kind: crate::layer::LayerKind) {
        let id = self.note_layer(kind);
        self.layer_stack.push(id);
    }

    fn pop_layers(&mut self, count: usize) {
        for _ in 0..count {
            self.layer_stack.pop();
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is one effect the layer stack may push for this node"
    )]
    fn push_effect_layers(
        &mut self,
        kind: Kind,
        abs: (f32, f32),
        w: f32,
        h: f32,
        radius: f32,
        alpha: Option<u8>,
        scroll: (f32, f32),
        transform: (f32, f32, f32, f32, f32),
        blur: Option<f32>,
        backdrop: Option<f32>,
        blend: Option<protocol::BlendMode>,
    ) -> usize {
        let mut pushed = 0;
        let identity = kind == Kind::Transform
            && transform.0 == 0.0
            && transform.1 == 0.0
            && transform.2 == 0.0
            && transform.3 == 1.0
            && transform.4 == 1.0;
        match kind {
            Kind::Scroll => {
                self.push_layer(crate::layer::LayerKind::Clip {
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                    corner_radius: 0.0,
                });
                self.push_layer(crate::layer::LayerKind::Offset {
                    x: -scroll.0,
                    y: -scroll.1,
                });
                pushed += 2;
            }
            Kind::Transform => {
                if !identity {
                    self.push_layer(crate::layer::LayerKind::Transform {
                        translate_x: transform.0,
                        translate_y: transform.1,
                        rotate_rad: transform.2,
                        scale_x: transform.3,
                        scale_y: transform.4,
                    });
                    pushed += 1;
                }
            }
            Kind::Opacity => {
                if alpha.unwrap_or(255) != 255 {
                    self.push_layer(crate::layer::LayerKind::Opacity {
                        alpha: alpha.unwrap_or(255),
                        x: abs.0,
                        y: abs.1,
                        w,
                        h,
                    });
                    pushed += 1;
                }
            }
            Kind::Clip => {
                self.push_layer(crate::layer::LayerKind::Clip {
                    x: abs.0,
                    y: abs.1,
                    w,
                    h,
                    corner_radius: radius,
                });
                pushed += 1;
            }
            _ => {}
        }
        if let Some(mode) = blend {
            self.push_layer(crate::layer::LayerKind::Blend {
                x: abs.0,
                y: abs.1,
                w,
                h,
                mode,
            });
            pushed += 1;
        }
        if let Some(sigma) = blur {
            self.push_layer(crate::layer::LayerKind::Blur {
                x: abs.0,
                y: abs.1,
                w,
                h,
                sigma,
            });
            pushed += 1;
        }
        if let Some(sigma) = backdrop {
            self.push_layer(crate::layer::LayerKind::BackdropBlur {
                x: abs.0,
                y: abs.1,
                w,
                h,
                sigma,
            });
            pushed += 1;
        }
        pushed
    }

    fn blit(&self, id: NodeId, out: &mut Vec<Command>, dx: f32, dy: f32, clip: Option<PaintClip>) {
        let Some(picture) = self.pictures.get(&id) else {
            return;
        };
        self.replay_commands(picture.commands.as_ref(), dx, dy, clip, out);
    }

    fn replay_commands(
        &self,
        commands: &[Command],
        dx: f32,
        dy: f32,
        clip: Option<PaintClip>,
        out: &mut Vec<Command>,
    ) {
        for command in commands {
            let moved = translate_command(command, dx, dy);
            if is_leaf_paint(&moved) && command_outside(&moved, clip) {
                continue;
            }
            out.push(moved);
        }
    }
}

fn translate_command(command: &Command, dx: f32, dy: f32) -> Command {
    if dx == 0.0 && dy == 0.0 {
        return command.clone();
    }
    match command.clone() {
        Command::Rect {
            x,
            y,
            w,
            h,
            corner_radius,
            color,
            stroke_width,
            stroke_color,
            gradient,
            shadow,
            box_shadows,
        } => Command::Rect {
            x: x + dx,
            y: y + dy,
            w,
            h,
            corner_radius,
            color,
            stroke_width,
            stroke_color,
            gradient,
            shadow,
            box_shadows,
        },
        Command::Text {
            x,
            y,
            max_width,
            font_size,
            paragraph,
            color,
            utf8,
            style,
            spans,
            links,
        } => Command::Text {
            x: x + dx,
            y: y + dy,
            max_width,
            font_size,
            paragraph,
            color,
            utf8,
            style,
            spans,
            links,
        },
        Command::Image {
            x,
            y,
            w,
            h,
            source,
            mode,
            required,
        } => Command::Image {
            x: x + dx,
            y: y + dy,
            w,
            h,
            source,
            mode,
            required,
        },
        Command::PushClip {
            x,
            y,
            w,
            h,
            corner_radius,
        } => Command::PushClip {
            x: x + dx,
            y: y + dy,
            w,
            h,
            corner_radius,
        },
        Command::PushOpacity { x, y, w, h, alpha } => Command::PushOpacity {
            x: x + dx,
            y: y + dy,
            w,
            h,
            alpha,
        },
        Command::PushBlur { x, y, w, h, sigma } => Command::PushBlur {
            x: x + dx,
            y: y + dy,
            w,
            h,
            sigma,
        },
        Command::PushBackdropBlur { x, y, w, h, sigma } => Command::PushBackdropBlur {
            x: x + dx,
            y: y + dy,
            w,
            h,
            sigma,
        },
        Command::PushBlend { x, y, w, h, mode } => Command::PushBlend {
            x: x + dx,
            y: y + dy,
            w,
            h,
            mode,
        },
        other => other,
    }
}

fn is_leaf_paint(command: &Command) -> bool {
    matches!(
        command,
        Command::Rect { .. } | Command::Text { .. } | Command::Image { .. }
    )
}

fn command_outside(command: &Command, clip: Option<PaintClip>) -> bool {
    match command {
        Command::Rect { x, y, w, h, .. }
        | Command::Image { x, y, w, h, .. }
        | Command::PushClip { x, y, w, h, .. }
        | Command::PushOpacity { x, y, w, h, .. }
        | Command::PushBlur { x, y, w, h, .. }
        | Command::PushBackdropBlur { x, y, w, h, .. }
        | Command::PushBlend { x, y, w, h, .. } => outside(clip, *x, *y, *w, *h),
        Command::Text {
            x, y, font_size, ..
        } => outside(clip, *x, *y, font_size.max(1.0), *font_size),
        Command::Fill { .. } | Command::PushTransform { .. } | Command::Pop => false,
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
    gradient: Option<protocol::Gradient>,
    shadow: Option<protocol::Shadow>,
    box_shadows: Arc<[protocol::BoxShadow]>,
}

pub(super) fn rect_paint(node: &super::node::Node) -> RectPaint {
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
    Paint(protocol::Gradient),
    Solid([u8; 4]),
    Drop,
}

pub(super) fn prepared_gradient(gradient: &protocol::Gradient) -> PreparedGradient {
    let stops = match gradient {
        protocol::Gradient::Linear { stops, .. }
        | protocol::Gradient::Radial { stops, .. }
        | protocol::Gradient::Sweep { stops, .. } => stops,
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
        protocol::Gradient::Linear { start, end, .. } => {
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
            PreparedGradient::Paint(protocol::Gradient::Linear {
                start: *start,
                end: *end,
                stops,
            })
        }
        protocol::Gradient::Radial { center, radius, .. } => {
            if !center[0].is_finite()
                || !center[1].is_finite()
                || !radius.is_finite()
                || *radius <= 0.0
            {
                return PreparedGradient::Drop;
            }
            PreparedGradient::Paint(protocol::Gradient::Radial {
                center: *center,
                radius: *radius,
                stops,
            })
        }
        protocol::Gradient::Sweep {
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
            PreparedGradient::Paint(protocol::Gradient::Sweep {
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
    use crate::MonoShaper;
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
