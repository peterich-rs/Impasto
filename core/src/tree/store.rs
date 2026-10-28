//! Slots, generations, and structural edits.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use taffy::prelude::*;

use super::layout::{align_axes, finite_number, flex_style, scroll_style, stack_style, tracks};
use super::node::{ImagePayload, Kind, MAX_GENERATION, Node, Payload, Slot, TextPayload};
use super::{Alignment, Discarded, Gesture, NodeId, TrackSize, Tree};
use crate::{Error, Shaper};

impl Tree {
    pub fn new(shaper: impl Shaper + 'static) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            taffy: TaffyTree::new(),
            shaper: Box::new(shaper),
            root: None,
            pending_discard: Vec::new(),
            arena: super::gesture::GestureArena::default(),
            focused: None,
            scopes: Vec::new(),
            motions: Vec::new(),
            motion_ends: Vec::new(),
            clock_ms: 0.0,
            applying_motion: false,
            dirty_boundaries: Vec::new(),
            pictures: HashMap::new(),
            runs: HashMap::new(),
            run: super::record::RunState::default(),
            finished_owners: Vec::new(),
            subtree_layers: HashMap::new(),
            layer_stack: Vec::new(),
            layer_tail: Vec::new(),
            layer_top: None,
            picture_writes: 0,
            layer_epoch: 0,
            layers: crate::layer::LayerTree::empty(),
            semantics_cache: HashMap::new(),
            image_counts: HashMap::new(),
            image_generation: 0,
            scroll_notices: Vec::new(),
            needs_layout: false,
            needs_paint: false,
            layout_passes: 0,
            pending_text: Vec::new(),
            sync_visited: 0,
            sync_synced: 0,
            sync_ms: 0.0,
            taffy_ms: 0.0,
            layout_work_ms: 0.0,
            layout_entries: 0,
            layout_cause: super::LayoutCause::None,
            text_reused: Cell::new(0),
            text_shaped: Cell::new(0),
            semantics_layout_seen: 0,
            scroll_semantics: Vec::new(),
            semantics_full: false,
            sweep_stack: Vec::new(),
            sweep_children: Vec::new(),
            text_scale: 1.0,
            laid_out: false,
            origin: (0.0, 0.0),
            viewport: None,
        }
    }

    pub fn row(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::Row, flex_style(FlexDirection::Row, gap))
    }

    pub fn column(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::Column, flex_style(FlexDirection::Column, gap))
    }

    pub fn grid(&mut self, columns: u16) -> Result<NodeId, Error> {
        let tracks = (0..columns).map(|_| min_content()).collect();
        self.container(
            Kind::Grid,
            Style {
                display: Display::Grid,
                grid_template_columns: tracks,
                align_items: Some(AlignItems::Start),
                justify_items: Some(AlignItems::Start),
                ..Style::default()
            },
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
        self.container(
            Kind::Grid,
            Style {
                display: Display::Grid,
                grid_template_columns: columns,
                grid_template_rows: rows,
                gap: Size {
                    width: length(gap),
                    height: length(gap),
                },
                align_items: Some(AlignItems::Stretch),
                justify_items: Some(AlignItems::Stretch),
                ..Style::default()
            },
        )
    }

    pub fn padding(&mut self, edges: f32) -> Result<NodeId, Error> {
        let edge = length(edges);
        self.container(
            Kind::Padding,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Stretch),
                padding: Rect {
                    left: edge,
                    right: edge,
                    top: edge,
                    bottom: edge,
                },
                ..Style::default()
            },
        )
    }

    pub fn align(&mut self, alignment: Alignment) -> Result<NodeId, Error> {
        let (cross, main) = align_axes(alignment);
        self.container(
            Kind::Align,
            Style {
                display: Display::Flex,
                align_items: Some(cross),
                justify_content: Some(main),
                ..Style::default()
            },
        )
    }

    pub fn sized(&mut self, width: f32, height: f32) -> Result<NodeId, Error> {
        self.container(
            Kind::Sized,
            Style {
                size: Size {
                    width: length(width),
                    height: length(height),
                },
                ..Style::default()
            },
        )
    }

    pub fn stack(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Stack, stack_style())
    }

    /// Clips descendants to this border box. A radius of zero is a rectangle.
    ///
    /// Hit testing uses the rounded outline. A point in the corner does not reach a child.
    pub fn clip(&mut self, corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(Kind::Clip, stack_style())?;
        self.node_mut(id)?
            .write_paint(|paint| paint.corner_radius = corner_radius);
        Ok(id)
    }

    /// Composites descendants as one layer. `alpha` is 0–255.
    pub fn opacity(&mut self, alpha: u8) -> Result<NodeId, Error> {
        let id = self.container(Kind::Opacity, stack_style())?;
        self.node_mut(id)?
            .write_paint(|paint| paint.alpha = Some(alpha));
        Ok(id)
    }

    /// A viewport. Children lay out at their full size and may overflow the box.
    ///
    /// The border box is the viewport. `set_scroll_offset` moves paint and hit
    /// testing only, so a scroll frame does not invalidate Taffy's layout cache.
    pub fn scroll(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Scroll, scroll_style())
    }

    /// A paint transform around the current origin. Layout ignores it.
    ///
    /// Hit testing inverts the same translate, clockwise rotate, and uniform scale
    /// the recorder pushes.
    ///
    /// The initial transform is identity: no translation, no rotation, scale 1.
    pub fn transform(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Transform, stack_style())
    }

    pub fn text(
        &mut self,
        utf8: impl Into<String>,
        font_size: f32,
        color: [u8; 4],
    ) -> Result<NodeId, Error> {
        let utf8 = utf8.into();
        let style = Style::default();
        let (index, generation) = self.reserve();
        let taffy = match self
            .taffy
            .new_leaf_with_context(style.clone(), index as usize)
        {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
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
        let style = Style::default();
        let (index, generation) = self.reserve();
        let taffy = match self
            .taffy
            .new_leaf_with_context(style.clone(), index as usize)
        {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
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
                    mode: protocol::ContentMode::Fill,
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
        let id = self.container(Kind::Decorated, flex_style(FlexDirection::Column, 0.0))?;
        let node = self.node_mut(id)?;
        node.write_paint(|paint| {
            paint.fill = Some(color);
            paint.corner_radius = corner_radius;
        });
        Ok(id)
    }

    pub fn pointer_region(&mut self) -> Result<NodeId, Error> {
        self.container(
            Kind::PointerRegion,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Stretch),
                justify_content: Some(JustifyContent::Start),
                ..Style::default()
            },
        )
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
                    Ok(())
                } else {
                    Err(Error::NotAChild)
                };
            }
            if self.node(anchor)?.parent != Some(parent) {
                return Err(Error::NotAChild);
            }
        }

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
        if let Ok(node) = self.node_mut(child) {
            node.flags.insert(super::node::NodeFlags::LAYOUT_DIRTY);
        }
        self.note_text_tree(child);
        self.mark_layout_from(parent);
        self.promote(parent);
        self.mark_structure();
        self.pending_discard.retain(|id| *id != child);
        Ok(())
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
            self.motions.retain(|motion| motion.node != *id);
        }
        self.mark_structure();
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
        let taffy = match self.taffy.new_leaf(style.clone()) {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
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
            self.slots[index as usize].node.as_mut().map(|node| {
                node.flags.insert(
                    super::node::NodeFlags::RELAYOUT_BOUNDARY
                        | super::node::NodeFlags::REPAINT_BOUNDARY,
                );
            });
        }
        id
    }

    pub(super) fn reserve(&mut self) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let generation = self.slots[index as usize].generation;
            (index, generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 1,
                node: None,
            });
            (index, 1)
        }
    }

    /// Drops whatever occupies `index` and recycles it until the generation cap.
    pub(super) fn retire(&mut self, index: u32) {
        let paint = self.slots.get_mut(index as usize).and_then(|slot| {
            slot.node.as_mut().and_then(|node| {
                node.text_mut().and_then(|text| {
                    text.metrics = None;
                    text.cached = None;
                    text.paragraph.take()
                })
            })
        });
        if let Some(id) = paint {
            self.shaper.release_paint(id);
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
        if self.root.is_some_and(|root| root.index == index) {
            self.root = None;
        }
        let generation = self.slots.get(index as usize).map(|slot| slot.generation);
        if let Some(generation) = generation {
            let id = NodeId { index, generation };
            self.pictures.remove(&id);
            self.runs.remove(&id);
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

    pub fn image_census(&self) -> crate::ImageCensus {
        crate::ImageCensus {
            generation: self.image_generation,
            counts: Arc::new(self.image_counts.clone()),
        }
    }

    pub fn layer_epoch(&self) -> u64 {
        self.layer_epoch
    }

    pub fn layer_tree(&self) -> crate::LayerTree {
        self.layers.clone()
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
        self.detach_links(child)
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
        let index = id.index as usize;
        let taffy_id = if matches!(kind, Kind::Text | Kind::Image) {
            self.taffy.new_leaf_with_context(style, index)?
        } else {
            self.taffy.new_leaf(style)?
        };
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
        self.laid_out = false;
        if let Some(id) = self.arena.hovered.take() {
            self.arena.pending.push(Gesture::Leave(id));
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let paints: Vec<protocol::ParagraphId> = self
            .slots
            .iter_mut()
            .filter_map(|slot| {
                slot.node.as_mut().and_then(|node| {
                    node.text_mut().and_then(|text| {
                        text.metrics = None;
                        text.cached = None;
                        text.paragraph.take()
                    })
                })
            })
            .collect();
        for id in paints {
            self.shaper.release_paint(id);
        }
    }
}

fn can_parent(kind: Kind) -> bool {
    !matches!(kind, Kind::Text | Kind::Image)
}

fn pins_children(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Stack | Kind::Clip | Kind::Opacity | Kind::Scroll | Kind::Transform
    )
}
