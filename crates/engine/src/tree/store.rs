//! Slots, generations, and structural edits.

use crate::geometry::MAX_GENERATION;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::prelude::*;

use super::layout::{
    align_axes, content_box, finite_number, flex_style, scroll_style_for, stack_style, tracks,
};
use super::node::{
    ImagePayload, Kind, Node, NodeFlags, Payload, Role, Slot, TextPayload, TransformGeom,
};
use super::{Alignment, Discarded, Gesture, NodeId, TextProbeTrace, TrackSize, Tree};
use crate::{Error, text::Shaper};

impl Tree {
    pub fn new(shaper: impl Shaper + 'static) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            taffy: super::LayoutStore::default(),
            shaper: Box::new(shaper),
            measure_open: false,
            root: None,
            pending_discard: Vec::new(),
            arena: super::gesture::PointerTable::default(),
            focused: None,
            scopes: Vec::new(),
            motions: Vec::new(),
            next_motion_id: 0,
            next_scroll_rule: 0,
            back_progress: 0.0,
            scroll_bindings: HashMap::new(),
            scroll_thresholds: Default::default(),
            motion_layers: HashMap::new(),
            list_compositor_windows: HashMap::new(),
            motion_ends: Vec::new(),
            clock_ms: 0.0,
            applying_motion: false,
            pictures: HashMap::new(),
            runs: HashMap::new(),
            run: super::record::RunState::default(),
            finished_owners: Vec::new(),
            subtree_layers: HashMap::new(),
            layer_stack: Vec::new(),
            layer_tail: Vec::new(),
            layer_top: None,
            picture_writes: 0,
            layer_record_visits: 0,
            layer_epoch: 0,
            canvas_geometry_epoch: 0,
            layers: crate::scene::LayerTree::empty(),
            shader_effects: HashMap::new(),
            semantics_cache: HashMap::new(),
            semantics_order: Vec::new(),
            platform_views: HashMap::new(),
            image_counts: HashMap::new(),
            image_generation: 0,
            scroll_notices: Vec::new(),
            scroll_paint_updates: 0,
            needs_layout: false,
            needs_paint: false,
            layout_passes: 0,
            pending_text: Vec::new(),
            changed_text: Vec::new(),
            sync_visited: 0,
            sync_synced: 0,
            sync_ms: 0.0,
            taffy_ms: 0.0,
            layout_work_ms: 0.0,
            layout_entries: 0,
            root_layouts: 0,
            root_layout_trigger: None,
            local_layouts: 0,
            same_text_layouts: 0,
            layout_cause: super::LayoutCause::None,
            pending_layout_cause: super::LayoutCause::None,
            text_reused: Cell::new(0),
            text_shaped: Cell::new(0),
            trace_text_probes: false,
            text_probes: TextProbeTrace::default(),
            semantics_layout_seen: 0,
            scroll_semantics: Vec::new(),
            semantics_full: false,
            semantics_enabled: true,
            autofill_enabled: false,
            autofill_nodes: Default::default(),
            sweep_stack: Vec::new(),
            sweep_children: Vec::new(),
            text_scale: 1.0,
            pixel_scale: 1.0,
            laid_out: false,
            origin: (0.0, 0.0),
            viewport: None,
            window: (0.0, 0.0),
            safe: (0.0, 0.0, 0.0, 0.0),
            refresh_pending: Vec::new(),
            debug: super::debug::DebugState::default(),
            style_nodes: HashMap::new(),
            style_queue: Vec::new(),
            style_source_owners: HashMap::new(),
            next_style_source: 0,
            style_applying: false,
            style_env: crate::style::Environment::default(),
            class_cache: crate::style::ClassCache::default(),
            style_gestures: Vec::new(),
        }
    }

    pub fn row(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::View, flex_style(FlexDirection::Row, gap))
    }

    pub fn column(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::View, flex_style(FlexDirection::Column, gap))
    }

    /// Column with min 0, shrink 0, and no parent max. Older constructors keep
    /// all constructors use the same sizing defaults.
    pub fn view(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::View, crate::style::defaults::view_style())
    }

    pub fn grid(&mut self, columns: u16) -> Result<NodeId, Error> {
        let tracks = (0..columns).map(|_| min_content()).collect();
        self.view_with_role(
            Style {
                display: Display::Grid,
                grid_template_columns: tracks,
                align_items: AlignItems::START,
                justify_items: AlignItems::START,
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                ..content_box()
            },
            Role::GRID,
        )
    }

    /// Grid with explicit column and row tracks. An empty list is [`Error::BadNumber`].
    pub fn grid_tracks(
        &mut self,
        columns: &[TrackSize],
        rows: &[TrackSize],
        gap: f32,
    ) -> Result<NodeId, Error> {
        finite_number(gap)?;
        if gap < 0.0 || columns.is_empty() || rows.is_empty() {
            return Err(Error::BadNumber);
        }
        let columns = tracks(columns)?;
        let rows = tracks(rows)?;
        self.view_with_role(
            Style {
                display: Display::Grid,
                grid_template_columns: columns,
                grid_template_rows: rows,
                gap: Size {
                    width: length(gap),
                    height: length(gap),
                },
                align_items: AlignItems::STRETCH,
                justify_items: AlignItems::STRETCH,
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                ..content_box()
            },
            Role::GRID,
        )
    }

    /// Inset wrapper. A column, so several children stack and each one stretches
    /// to the inner width. A row here laid siblings out sideways and left the
    /// single child at its content width.
    pub fn padding(&mut self, edges: f32) -> Result<NodeId, Error> {
        let edge = length(edges);
        self.container(
            Kind::View,
            Style {
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                align_items: AlignItems::STRETCH,
                padding: Rect {
                    left: edge,
                    right: edge,
                    top: edge,
                    bottom: edge,
                },
                ..content_box()
            },
        )
    }

    pub fn align(&mut self, alignment: Alignment) -> Result<NodeId, Error> {
        let (cross, main) = align_axes(alignment);
        self.container(
            Kind::View,
            Style {
                display: Display::Flex,
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                align_items: cross,
                justify_content: main,
                ..content_box()
            },
        )
    }

    pub fn sized(&mut self, width: f32, height: f32) -> Result<NodeId, Error> {
        let id = self.container(
            Kind::View,
            Style {
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                size: Size {
                    width: length(width),
                    height: length(height),
                },
                ..content_box()
            },
        )?;
        self.node_mut(id)?
            .flags
            .insert(NodeFlags::RELAYOUT_BOUNDARY);
        Ok(id)
    }

    pub fn stack(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Stack, stack_style())
    }

    /// Clips descendants to this border box. A radius of zero is a rectangle.
    ///
    /// Hit testing uses the rounded outline. A point in the corner does not reach a child.
    pub fn clip(&mut self, corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(Kind::Stack, stack_style())?;
        let node = self.node_mut(id)?;
        node.role.insert(Role::CLIP);
        node.flags.insert(NodeFlags::REPAINT_BOUNDARY);
        node.write_paint(|paint| paint.corner_radius = corner_radius);
        Ok(id)
    }

    /// Composites descendants as one layer. `alpha` is 0–255.
    pub fn opacity(&mut self, alpha: u8) -> Result<NodeId, Error> {
        let id = self.container(Kind::Stack, stack_style())?;
        let node = self.node_mut(id)?;
        node.flags.insert(NodeFlags::REPAINT_BOUNDARY);
        node.write_paint(|paint| paint.alpha = Some(alpha));
        Ok(id)
    }

    /// A viewport. Children lay out at their full size and may overflow the box.
    ///
    /// The border box is the viewport. `set_scroll_offset` moves paint and hit
    /// testing only, so a scroll frame does not invalidate Taffy's layout cache.
    /// The axis is [`crate::motion::ScrollAxis::Both`].
    pub fn scroll(&mut self) -> Result<NodeId, Error> {
        self.scroll_with(crate::motion::ScrollAxis::Both)
    }

    /// [`Tree::scroll`] locked to `axis` from the start.
    pub fn scroll_with(&mut self, axis: crate::motion::ScrollAxis) -> Result<NodeId, Error> {
        let id = self.container(Kind::Scroll, scroll_style_for(axis))?;
        if let Some(geom) = self.node_mut(id)?.scroll_mut() {
            geom.axis = axis;
        }
        Ok(id)
    }

    /// A paint transform around the current origin. Layout ignores it.
    ///
    /// Hit testing inverts the same translate, clockwise rotate, and uniform scale
    /// the recorder pushes.
    ///
    /// The initial transform is identity: no translation, no rotation, scale 1.
    pub fn transform(&mut self) -> Result<NodeId, Error> {
        let id = self.container(Kind::Stack, stack_style())?;
        let node = self.node_mut(id)?;
        node.role.insert(Role::TRANSFORM);
        node.flags.insert(NodeFlags::REPAINT_BOUNDARY);
        node.transform = Some(Box::new(TransformGeom::identity()));
        Ok(id)
    }

    pub fn text(
        &mut self,
        utf8: impl Into<String>,
        font_size: f32,
        color: [u8; 4],
    ) -> Result<NodeId, Error> {
        let utf8 = utf8.into();
        let style = content_box();
        let (index, generation) = self.reserve();
        let taffy = self
            .taffy
            .new_node(style.clone(), NodeId { index, generation }, true);
        let id = self.occupy(
            index,
            generation,
            Node::new(
                Kind::Text,
                style,
                taffy,
                Payload::Text(Box::new(TextPayload::new(utf8, font_size, color))),
            ),
        );
        self.note_text(id);
        Ok(id)
    }

    pub fn image(
        &mut self,
        source: impl Into<String>,
        width: f32,
        height: f32,
    ) -> Result<NodeId, Error> {
        let style = content_box();
        let (index, generation) = self.reserve();
        let taffy = self
            .taffy
            .new_node(style.clone(), NodeId { index, generation }, true);
        let source = Arc::<str>::from(source.into());
        let id = self.occupy(
            index,
            generation,
            Node::new(
                Kind::Image,
                style,
                taffy,
                Payload::Image(ImagePayload {
                    source: Arc::clone(&source),
                    width,
                    height,
                    mode: crate::scene::ContentMode::Fill,
                    required: false,
                }),
            ),
        );
        self.note_image(&source, 1);
        Ok(id)
    }

    /// Paints its border box and sizes itself to its children.
    ///
    /// `corner_radius` is in scene points. Zero paints a sharp rectangle and does not
    /// change hit testing.
    pub fn decorated(&mut self, color: [u8; 4], corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(Kind::View, flex_style(FlexDirection::Column, 0.0))?;
        let node = self.node_mut(id)?;
        node.write_paint(|paint| {
            paint.fill = Some(color);
            paint.corner_radius = corner_radius;
        });
        Ok(id)
    }

    /// A view with pointer listeners participates in target selection.
    pub fn set_pointer_target(&mut self, id: NodeId) -> Result<(), Error> {
        self.node_mut(id)?.role.insert(Role::POINTER);
        Ok(())
    }

    pub fn pointer_region(&mut self) -> Result<NodeId, Error> {
        self.view_with_role(
            Style {
                display: Display::Flex,
                flex_shrink: 0.0,
                min_size: crate::style::defaults::zero_min(),
                align_items: AlignItems::STRETCH,
                justify_content: JustifyContent::START,
                ..content_box()
            },
            Role::POINTER,
        )
    }

    fn view_with_role(&mut self, style: Style, role: Role) -> Result<NodeId, Error> {
        let id = self.container(Kind::View, style)?;
        self.node_mut(id)?.role.insert(role);
        Ok(id)
    }

    /// Moves every child of `from` onto `to`. The placeholder stays put.
    pub fn reparent_children(&mut self, from: NodeId, to: NodeId) -> Result<(), Error> {
        self.node(from)?;
        self.node(to)?;
        let mut child = self.node(from)?.first_child;
        let mut kids = Vec::new();
        while let Some(current) = child {
            kids.push(current);
            child = self.node(current)?.next;
        }
        for id in kids {
            self.insert(to, id, None)?;
        }
        Ok(())
    }

    pub fn add_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        self.insert(parent, child, None)
    }

    /// Places `child` under `parent`, before `before` when that anchor is set.
    ///
    /// A child that already has a parent moves. Taffy nodes that are still
    /// attached to the scene node stay; `revive` runs only when they were released.
    pub fn insert(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), Error> {
        let before = match self.insert_target(parent, child, before)? {
            InsertPlan::AlreadyHere => return Ok(()),
            InsertPlan::Place { before } => before,
        };

        let previous_root = self.root;
        if self.node(parent)?.taffy.is_none() {
            self.revive(parent)?;
        }
        if self.node(child)?.taffy.is_none() {
            self.revive(child)?;
        }
        if self.node(child)?.parent.is_some() {
            self.unlink(child)?;
        } else if self.root == Some(child) {
            self.root = None;
        }

        self.pin_stack_child(parent, child)?;
        let index = match before {
            Some(anchor) => self.child_index(parent, anchor)?,
            None => self.child_count(parent)?,
        };
        let parent_taffy = self.node(parent)?.taffy.ok_or(Error::UnknownNode)?;
        let child_taffy = self.node(child)?.taffy.ok_or(Error::UnknownNode)?;
        self.taffy
            .insert_child_at_index(parent_taffy, index, child_taffy)?;
        self.attach_links(parent, child, before)?;
        self.invalidate_subtree_layers(parent);
        if let Ok(node) = self.node_mut(child) {
            node.flags.insert(NodeFlags::LAYOUT_DIRTY);
        }
        self.note_text_tree(child);
        self.mark_layout_from(parent);
        self.promote(parent);
        if previous_root != self.root {
            self.laid_out = false;
        }
        self.mark_structure();
        self.pending_discard.retain(|id| *id != child);
        Ok(())
    }

    fn insert_target(
        &self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<InsertPlan, Error> {
        if parent == child || self.is_under(parent, child)? {
            return Err(Error::NotAChild);
        }
        if !can_parent(self.node(parent)?.kind) {
            return Err(Error::NotAParent);
        }
        let _ = self.node(child)?;
        // A sibling removed earlier in this turn is not a usable anchor.
        // Appending keeps the insert from failing while that id is still live.
        let before = match before {
            Some(anchor) if anchor != child && self.pending_discard.contains(&anchor) => None,
            other => other,
        };
        if let Some(anchor) = before {
            if anchor == child {
                return if self.node(child)?.parent == Some(parent) {
                    Ok(InsertPlan::AlreadyHere)
                } else {
                    Err(Error::NotAChild)
                };
            }
            if self.node(anchor)?.parent != Some(parent) {
                return Err(Error::NotAChild);
            }
        }
        Ok(InsertPlan::Place { before })
    }

    /// Detaches `child` from `parent` and queues it for [`Tree::discard_detached`].
    ///
    /// Taffy nodes, paint paragraphs, and picture cache entries stay with the
    /// scene ids. A later [`Tree::insert`] drops the id from the queue and does
    /// not rebuild Taffy. A focused node in the subtree produces [`Gesture::Blur`].
    /// Focus scopes inside the subtree are popped. Handlers on a detached region
    /// are never hit.
    pub fn remove(&mut self, parent: NodeId, child: NodeId) -> Result<Vec<Gesture>, Error> {
        if self.pending_discard.contains(&child) {
            return Ok(Vec::new());
        }
        if self.node(child)?.parent != Some(parent) {
            return Err(Error::NotAChild);
        }
        let gestures = self.unlink_kept(parent, child)?;
        self.pending_discard.push(child);
        Ok(gestures)
    }

    /// Detaches `child` without queueing [`Tree::discard_detached`].
    ///
    /// The subtree keeps its Taffy nodes and ids across frames. [`Tree::discard`]
    /// is what retires it. A node already queued by [`Tree::remove`] is taken off
    /// that queue. A focused node still produces [`Gesture::Blur`].
    pub fn detach(&mut self, parent: NodeId, child: NodeId) -> Result<Vec<Gesture>, Error> {
        let _ = self.node(child)?;
        let gestures = if self.node(child)?.parent == Some(parent) {
            self.unlink_kept(parent, child)?
        } else if self.node(child)?.parent.is_none() {
            Vec::new()
        } else {
            return Err(Error::NotAChild);
        };
        self.pending_discard.retain(|id| *id != child);
        Ok(gestures)
    }

    fn unlink_kept(&mut self, parent: NodeId, child: NodeId) -> Result<Vec<Gesture>, Error> {
        self.drop_scopes_under(child);
        self.unlink(child)?;
        let mut gestures = self.blur_contained(child);
        self.drop_routes_in(child);
        self.mark_layout_from(parent);
        self.mark_structure();
        gestures.extend(self.drain_pending());
        Ok(gestures)
    }

    fn drop_scopes_under(&mut self, root: NodeId) {
        let scopes = self.scopes.clone();
        self.scopes = scopes
            .into_iter()
            .filter(|scope| !self.contains(root, *scope))
            .collect();
    }

    /// Retires `id` and every descendant. The caller guarantees `id` has no parent.
    ///
    /// `remove` leaves the slots so the subtree can be inserted again. `discard`
    /// is the step that makes those ids unknown. The returned ids are the ones
    /// that were retired, children before their parent. A focused node in the
    /// subtree also produces [`Gesture::Blur`].
    pub fn discard(&mut self, id: NodeId) -> Result<Discarded, Error> {
        if self.node(id)?.parent.is_some() {
            return Err(Error::NotAChild);
        }
        let was_root = self.root == Some(id);
        self.pending_discard.retain(|pending| *pending != id);
        self.drop_scopes_under(id);
        let gestures = self.blur_contained(id);
        self.drop_routes_in(id);
        if self.node(id)?.taffy.is_some() {
            self.release_taffy(id)?;
        }
        let mut ids = Vec::new();
        self.retire_postorder(id, &mut ids)?;
        for id in &ids {
            self.platform_views.remove(id);
            self.autofill_nodes.remove(id);
            self.motions.retain(|motion| motion.node != *id);
        }
        if was_root {
            self.laid_out = false;
            self.mark_structure();
        }
        let mut gestures = gestures;
        gestures.extend(self.drain_pending());
        Ok(Discarded { ids, gestures })
    }

    /// Retires subtrees [`Tree::remove`] detached that were not inserted again.
    ///
    /// A child inserted before this call is left in place. Ids already retired
    /// by an earlier [`Tree::discard`] are skipped.
    pub fn discard_detached(&mut self) -> Discarded {
        let pending = std::mem::take(&mut self.pending_discard);
        let mut ids = Vec::new();
        let mut gestures = Vec::new();
        for id in pending {
            let parentless = self.node(id).ok().is_some_and(|node| node.parent.is_none());
            if !parentless {
                continue;
            }
            let Ok(retired) = self.discard(id) else {
                continue;
            };
            ids.extend(retired.ids);
            gestures.extend(retired.gestures);
        }
        Discarded { ids, gestures }
    }

    /// Queues a parentless non-root node for retirement unless it is inserted again.
    pub fn discard_if_unattached(&mut self, id: NodeId) {
        if self.root != Some(id) && self.node(id).is_ok_and(|node| node.parent.is_none()) {
            self.pending_discard.push(id);
        }
    }

    pub(super) fn retire_postorder(
        &mut self,
        id: NodeId,
        retired: &mut Vec<NodeId>,
    ) -> Result<(), Error> {
        let mut child = self.node(id)?.first_child;
        while let Some(current) = child {
            let next = self.node(current)?.next;
            self.retire_postorder(current, retired)?;
            child = next;
        }
        retired.push(id);
        self.retire(id.index);
        Ok(())
    }

    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.parent)
    }

    pub fn first_child(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.first_child)
    }

    pub fn next_sibling(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.next)
    }

    pub(super) fn container(&mut self, kind: Kind, style: Style) -> Result<NodeId, Error> {
        let (index, generation) = self.reserve();
        let taffy = self
            .taffy
            .new_node(style.clone(), NodeId { index, generation }, false);
        Ok(self.occupy(
            index,
            generation,
            Node::new(kind, style, taffy, Payload::Container),
        ))
    }

    /// The first node becomes the root so a one-node tree can lay out. Moving
    /// that node under a parent promotes the new top.
    pub(super) fn occupy(&mut self, index: u32, generation: u32, node: Node) -> NodeId {
        self.slots[index as usize].node = Some(node);
        let id = NodeId { index, generation };
        if self.root.is_none() {
            self.root = Some(id);
            if let Some(node) = self.slots[index as usize].node.as_mut() {
                node.flags
                    .insert(NodeFlags::RELAYOUT_BOUNDARY | NodeFlags::REPAINT_BOUNDARY);
            }
        }
        id
    }

    pub(super) fn reserve(&mut self) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let generation = self.slots[index as usize].generation;
            (index, generation)
        } else {
            let index = crate::cast::u32_from_usize(self.slots.len());
            self.slots.push(Slot {
                generation: 1,
                node: None,
            });
            (index, 1)
        }
    }

    /// Drops whatever occupies `index` and recycles it until the generation cap.
    pub(super) fn retire(&mut self, index: u32) {
        if let Some(slot) = self.slots.get_mut(index as usize)
            && let Some(node) = slot.node.as_mut()
            && let Some(text) = node.text_mut()
        {
            text.metrics = None;
            text.cached = None;
            text.painted = None;
        }
        if let Some(source) = self
            .slots
            .get(index as usize)
            .and_then(|slot| slot.node.as_ref())
            .and_then(|node| node.image())
            .map(|image| Arc::clone(&image.source))
        {
            self.note_image(&source, -1);
        }
        if let Some(list) = self
            .slots
            .get_mut(index as usize)
            .and_then(|slot| slot.node.as_mut())
            .and_then(|node| node.canvas.take())
        {
            self.note_canvas_images(&list, -1);
        }
        if self.root.is_some_and(|root| root.index == index) {
            self.root = None;
        }
        self.drop_style(index);
        let generation = self.slots.get(index as usize).map(|slot| slot.generation);
        if let Some(generation) = generation {
            let id = NodeId { index, generation };
            self.remove_shader_effect(id);
            self.forget_scroll_rules(id);
            self.pictures.remove(&id);
            self.runs.remove(&id);
            self.subtree_layers.remove(&id);
        }
        self.arena.clear_index(index);
        if self.focused.is_some_and(|focused| focused.index == index) {
            self.focused = None;
        }
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        slot.node = None;
        if slot.generation >= MAX_GENERATION {
            return;
        }
        slot.generation += 1;
        self.free.push(index);
    }

    #[cfg(feature = "js")]
    pub(crate) fn has_node(&self, id: NodeId) -> bool {
        self.node(id).is_ok()
    }

    pub(super) fn node(&self, id: NodeId) -> Result<&Node, Error> {
        let slot = self.slots.get(id.slot()).ok_or(Error::UnknownNode)?;
        if slot.generation != id.generation {
            return Err(Error::UnknownNode);
        }
        slot.node.as_ref().ok_or(Error::UnknownNode)
    }

    pub(super) fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, Error> {
        let slot = self.slots.get_mut(id.slot()).ok_or(Error::UnknownNode)?;
        if slot.generation != id.generation {
            return Err(Error::UnknownNode);
        }
        slot.node.as_mut().ok_or(Error::UnknownNode)
    }

    pub(super) fn get(&self, id: NodeId) -> Option<&Node> {
        self.node(id).ok()
    }

    /// Every image still attached to the tree, including children of a zero-size clip.
    pub fn image_source_counts(&self) -> HashMap<Arc<str>, u32> {
        self.image_counts.clone()
    }

    pub fn image_generation(&self) -> u64 {
        self.image_generation
    }

    pub fn image_census(&self) -> crate::semantics::ImageCensus {
        crate::semantics::ImageCensus {
            generation: self.image_generation,
            counts: Arc::new(self.image_counts.clone()),
        }
    }

    pub fn layer_epoch(&self) -> u64 {
        self.layer_epoch
    }

    pub fn layer_tree(&self) -> crate::scene::LayerTree {
        self.layers.clone()
    }

    pub(super) fn note_canvas_images(&mut self, list: &crate::scene::CanvasList, delta: i32) {
        for image in list.images() {
            self.note_image(&image.source, delta);
        }
    }

    pub(super) fn note_image(&mut self, source: &Arc<str>, delta: i32) {
        if source.is_empty() || delta == 0 {
            return;
        }
        if delta > 0 {
            *self.image_counts.entry(Arc::clone(source)).or_insert(0) += delta as u32;
        } else if let Some(count) = self.image_counts.get_mut(source) {
            *count = count.saturating_sub((-delta) as u32);
            if *count == 0 {
                self.image_counts.remove(source);
            }
        }
        self.image_generation = self.image_generation.wrapping_add(1);
    }

    /// Siblings with a non-zero `z_index`, in paint order. `hit` reverses that
    /// order. `None` means every sibling is zero and the linked list is enough.
    pub(super) fn ordered_children(&self, id: NodeId, hit: bool) -> Option<Vec<NodeId>> {
        let first = self.get(id).and_then(|node| node.first_child);
        let mut cursor = first;
        loop {
            let current = self.get(cursor?)?;
            if current.z_index != 0 {
                break;
            }
            cursor = current.next;
        }
        let mut child = first;
        let mut rows = Vec::new();
        while let Some(current) = child {
            let Some(node) = self.get(current) else {
                break;
            };
            rows.push((node.z_index, current));
            child = node.next;
        }
        rows.sort_by_key(|(z, _)| *z);
        let mut ids: Vec<NodeId> = rows.into_iter().map(|(_, id)| id).collect();
        if hit {
            ids.reverse();
        }
        Some(ids)
    }

    pub(super) fn is_under(&self, mut node: NodeId, ancestor: NodeId) -> Result<bool, Error> {
        let mut guard = self.slots.len() + 1;
        loop {
            if node == ancestor {
                return Ok(true);
            }
            if guard == 0 {
                return Err(Error::NotAChild);
            }
            guard -= 1;
            let Some(parent) = self.node(node)?.parent else {
                return Ok(false);
            };
            node = parent;
        }
    }

    pub(super) fn unlink(&mut self, child: NodeId) -> Result<(), Error> {
        let Some(parent) = self.node(child)?.parent else {
            return Ok(());
        };
        let parent_taffy = self.node(parent)?.taffy;
        let child_taffy = self.node(child)?.taffy;
        if let (Some(parent_taffy), Some(child_taffy)) = (parent_taffy, child_taffy) {
            self.taffy.remove_child(parent_taffy, child_taffy)?;
        }
        self.detach_links(child)?;
        self.invalidate_subtree_layers(parent);
        Ok(())
    }

    pub(super) fn child_count(&self, id: NodeId) -> Result<usize, Error> {
        let mut count = 0;
        let mut child = self.node(id)?.first_child;
        while let Some(current) = child {
            count += 1;
            child = self.node(current)?.next;
        }
        Ok(count)
    }

    pub(super) fn child_index(&self, parent: NodeId, child: NodeId) -> Result<usize, Error> {
        let mut index = 0;
        let mut current = self.node(parent)?.first_child;
        while let Some(id) = current {
            if id == child {
                return Ok(index);
            }
            index += 1;
            current = self.node(id)?.next;
        }
        Err(Error::NotAChild)
    }

    /// `child` is already detached. Returns the Taffy insert index.
    pub(super) fn attach_links(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<usize, Error> {
        let index = match before {
            Some(anchor) => self.child_index(parent, anchor)?,
            None => self.child_count(parent)?,
        };
        let prev = match before {
            Some(anchor) => self.node(anchor)?.prev,
            None => self.node(parent)?.last_child,
        };
        {
            let node = self.node_mut(child)?;
            node.parent = Some(parent);
            node.prev = prev;
            node.next = before;
        }
        if let Some(prev) = prev {
            self.node_mut(prev)?.next = Some(child);
        } else {
            self.node_mut(parent)?.first_child = Some(child);
        }
        if let Some(anchor) = before {
            self.node_mut(anchor)?.prev = Some(child);
        } else {
            self.node_mut(parent)?.last_child = Some(child);
        }
        Ok(index)
    }

    pub(super) fn detach_links(&mut self, child: NodeId) -> Result<(), Error> {
        let (parent, prev, next) = {
            let node = self.node(child)?;
            (node.parent, node.prev, node.next)
        };
        if let Some(prev) = prev {
            self.node_mut(prev)?.next = next;
        } else if let Some(parent) = parent {
            self.node_mut(parent)?.first_child = next;
        }
        if let Some(next) = next {
            self.node_mut(next)?.prev = prev;
        } else if let Some(parent) = parent {
            self.node_mut(parent)?.last_child = prev;
        }
        let node = self.node_mut(child)?;
        node.parent = None;
        node.prev = None;
        node.next = None;
        Ok(())
    }

    pub(super) fn pin_stack_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        if !pins_children(self.node(parent)?.kind) {
            return Ok(());
        }
        let mut style = self.node(child)?.style.clone();
        style.grid_row = line(1);
        style.grid_column = line(1);
        self.write_style(child, style)
    }

    pub(super) fn revive(&mut self, id: NodeId) -> Result<(), Error> {
        if self.node(id)?.taffy.is_some() {
            return Ok(());
        }
        let mut child = self.node(id)?.first_child;
        let mut children = Vec::new();
        while let Some(current) = child {
            children.push(current);
            child = self.node(current)?.next;
        }
        for child in &children {
            self.revive(*child)?;
        }
        let style = self.node(id)?.style.clone();
        let kind = self.node(id)?.kind;
        let taffy_id = self
            .taffy
            .new_node(style, id, matches!(kind, Kind::Text | Kind::Image));
        if !children.is_empty() {
            let mut child_ids = Vec::with_capacity(children.len());
            for child in &children {
                child_ids.push(self.node(*child)?.taffy.ok_or(Error::UnknownNode)?);
            }
            self.taffy.set_children(taffy_id, &child_ids)?;
        }
        self.node_mut(id)?.taffy = Some(taffy_id);
        Ok(())
    }

    pub(super) fn release_taffy(&mut self, id: NodeId) -> Result<(), Error> {
        let mut child = self.node(id)?.first_child;
        while let Some(current) = child {
            let next = self.node(current)?.next;
            self.release_taffy(current)?;
            child = next;
        }
        let taffy_id = self.node_mut(id)?.taffy.take();
        if let Some(taffy_id) = taffy_id {
            self.taffy.remove(taffy_id)?;
        }
        Ok(())
    }

    pub(super) fn promote(&mut self, parent: NodeId) {
        let top = self.top(parent);
        match self.root {
            None => self.root = Some(top),
            Some(root) => {
                let rooted = self
                    .node(root)
                    .ok()
                    .is_some_and(|node| node.parent.is_some());
                if rooted {
                    self.root = Some(top);
                }
            }
        }
    }

    pub(super) fn top(&self, mut id: NodeId) -> NodeId {
        while let Some(parent) = self.get(id).and_then(|node| node.parent) {
            id = parent;
        }
        id
    }

    pub(super) fn mark_structure(&mut self) {
        self.note_layout(super::LayoutCause::Structure);
        self.mark_layout();
        if let Some(id) = self.arena.hovered.take() {
            self.arena.pending.push(Gesture::Leave(id));
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.finish_measure();
    }
}

enum InsertPlan {
    AlreadyHere,
    Place { before: Option<NodeId> },
}

fn can_parent(kind: Kind) -> bool {
    !matches!(kind, Kind::Text | Kind::Image)
}

fn pins_children(kind: Kind) -> bool {
    matches!(kind, Kind::Stack | Kind::Scroll)
}
