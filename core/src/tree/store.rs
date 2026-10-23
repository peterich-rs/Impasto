//! Slots, generations, and structural edits.

use std::cell::Cell;

use protocol::TextStyle;
use taffy::prelude::*;

use super::layout::{align_axes, finite_number, flex_style, scroll_style, stack_style, tracks};
use super::node::{Hit, ImageNode, Kind, MAX_GENERATION, Node, Slot, TextNode};
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
            pressed: None,
            secondary: None,
            focused: None,
            hovered: None,
            pending: Vec::new(),
            drag: None,
            dirty: false,
            laid_out: false,
            origin: (0.0, 0.0),
            viewport: None,
        }
    }

    pub fn row(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::Row, Hit::Defer, flex_style(FlexDirection::Row, gap))
    }

    pub fn column(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(
            Kind::Column,
            Hit::Defer,
            flex_style(FlexDirection::Column, gap),
        )
    }

    pub fn grid(&mut self, columns: u16) -> Result<NodeId, Error> {
        let tracks = (0..columns).map(|_| min_content()).collect();
        self.container(
            Kind::Grid,
            Hit::Defer,
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
            Hit::Defer,
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
            Hit::Defer,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Start),
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
            Hit::Defer,
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
            Hit::Defer,
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
        self.container(Kind::Stack, Hit::Defer, stack_style())
    }

    /// Clips descendants to this border box. A radius of zero is a rectangle.
    ///
    /// Hit testing uses that rectangle. The rounded outline does not change hits.
    pub fn clip(&mut self, corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(Kind::Clip, Hit::Defer, stack_style())?;
        self.node_mut(id)?.corner_radius = corner_radius;
        Ok(id)
    }

    /// Composites descendants as one layer. `alpha` is 0–255.
    pub fn opacity(&mut self, alpha: u8) -> Result<NodeId, Error> {
        let id = self.container(Kind::Opacity, Hit::Defer, stack_style())?;
        self.node_mut(id)?.alpha = Some(alpha);
        Ok(id)
    }

    /// A viewport. Children lay out at their full size and may overflow the box.
    ///
    /// The border box is the viewport. `set_scroll_offset` moves paint and hit
    /// testing only, so a scroll frame does not invalidate Taffy's layout cache.
    pub fn scroll(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Scroll, Hit::Defer, scroll_style())
    }

    /// A paint transform around the current origin. Layout and hit testing ignore it.
    ///
    /// The initial transform is identity: no translation, no rotation, scale 1.
    pub fn transform(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Transform, Hit::Defer, stack_style())
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
        Ok(self.occupy(
            index,
            generation,
            Node {
                kind: Kind::Text,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit: Hit::Opaque,
                text: Some(TextNode {
                    utf8,
                    font_size,
                    color,
                    max_width: Cell::new(0.0),
                    style: TextStyle::default(),
                }),
                image: None,
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
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
        Ok(self.occupy(
            index,
            generation,
            Node {
                kind: Kind::Image,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit: Hit::Opaque,
                text: None,
                image: Some(ImageNode {
                    source: source.into(),
                    width,
                    height,
                }),
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
    }

    /// Paints its border box and sizes itself to its children.
    ///
    /// `corner_radius` is in scene points. Zero paints a sharp rectangle and does not
    /// change hit testing.
    pub fn decorated(&mut self, color: [u8; 4], corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(
            Kind::Decorated,
            Hit::Defer,
            flex_style(FlexDirection::Column, 0.0),
        )?;
        let node = self.node_mut(id)?;
        node.fill = Some(color);
        node.corner_radius = corner_radius;
        Ok(id)
    }

    pub fn pointer_region(&mut self) -> Result<NodeId, Error> {
        self.container(
            Kind::PointerRegion,
            Hit::Region,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Start),
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
    /// A child that already has a parent moves. A child whose Taffy nodes were
    /// released is rebuilt from the stored style and child list.
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
            Some(anchor) => self
                .node(parent)?
                .children
                .iter()
                .position(|id| *id == anchor)
                .ok_or(Error::NotAChild)?,
            None => self.node(parent)?.children.len(),
        };
        let parent_taffy = self.node(parent)?.taffy.ok_or(Error::UnknownNode)?;
        let child_taffy = self.node(child)?.taffy.ok_or(Error::UnknownNode)?;
        self.taffy
            .insert_child_at_index(parent_taffy, index, child_taffy)?;
        self.node_mut(parent)?.children.insert(index, child);
        self.node_mut(child)?.parent = Some(parent);
        self.promote(parent);
        self.mark_structure();
        Ok(())
    }

    /// Detaches `child` from `parent` and releases the subtree's Taffy nodes.
    ///
    /// The scene nodes stay, with the same ids, so the subtree can be inserted
    /// again. Handlers on a detached region are never hit. A focused node in
    /// the subtree produces [`Gesture::Blur`]. Inserting elsewhere does not.
    pub fn remove(&mut self, parent: NodeId, child: NodeId) -> Result<Vec<Gesture>, Error> {
        if self.node(child)?.parent != Some(parent) {
            return Err(Error::NotAChild);
        }
        self.node_mut(parent)?.children.retain(|id| *id != child);
        self.node_mut(child)?.parent = None;
        self.release_taffy(child)?;
        let mut gestures = self.blur_contained(child);
        self.mark_structure();
        gestures.extend(self.drain_pending());
        Ok(gestures)
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
        let gestures = self.blur_contained(id);
        if self.node(id)?.taffy.is_some() {
            self.release_taffy(id)?;
        }
        let mut ids = Vec::new();
        self.retire_postorder(id, &mut ids)?;
        if self
            .secondary
            .as_ref()
            .is_some_and(|press| ids.contains(&press.target))
        {
            self.secondary = None;
        }
        self.mark_structure();
        let mut gestures = gestures;
        gestures.extend(self.drain_pending());
        Ok(Discarded { ids, gestures })
    }

    pub(super) fn retire_postorder(
        &mut self,
        id: NodeId,
        retired: &mut Vec<NodeId>,
    ) -> Result<(), Error> {
        let children = self.node(id)?.children.clone();
        for child in children {
            self.retire_postorder(child, retired)?;
        }
        retired.push(id);
        self.retire(id.index);
        Ok(())
    }

    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.parent)
    }

    pub fn first_child(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.children.first().copied())
    }

    pub fn next_sibling(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        let Some(parent) = self.node(id)?.parent else {
            return Ok(None);
        };
        let children = &self.node(parent)?.children;
        let Some(index) = children.iter().position(|child| *child == id) else {
            return Ok(None);
        };
        Ok(children.get(index + 1).copied())
    }

    pub(super) fn container(
        &mut self,
        kind: Kind,
        hit: Hit,
        style: Style,
    ) -> Result<NodeId, Error> {
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
            Node {
                kind,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit,
                text: None,
                image: None,
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
    }

    /// The first node becomes the root so a one-node tree can lay out. Moving
    /// that node under a parent promotes the new top.
    pub(super) fn occupy(&mut self, index: u32, generation: u32, node: Node) -> NodeId {
        self.slots[index as usize].node = Some(node);
        let id = NodeId { index, generation };
        if self.root.is_none() {
            self.root = Some(id);
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
        if self.root.is_some_and(|root| root.index == index) {
            self.root = None;
        }
        if self.pressed.is_some_and(|pressed| pressed.index == index) {
            self.pressed = None;
        }
        if self.focused.is_some_and(|focused| focused.index == index) {
            self.focused = None;
        }
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.scroll.is_some_and(|scroll| scroll.index == index))
        {
            self.drag = None;
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
        self.node_mut(parent)?.children.retain(|id| *id != child);
        let parent_taffy = self.node(parent)?.taffy;
        let child_taffy = self.node(child)?.taffy;
        if let (Some(parent_taffy), Some(child_taffy)) = (parent_taffy, child_taffy) {
            self.taffy.remove_child(parent_taffy, child_taffy)?;
        }
        self.node_mut(child)?.parent = None;
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
        let children = self.node(id)?.children.clone();
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
        let children = self.node(id)?.children.clone();
        for child in children {
            self.release_taffy(child)?;
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
        self.dirty = true;
        self.laid_out = false;
        self.pressed = None;
        self.drag = None;
        if let Some(id) = self.hovered.take() {
            self.pending.push(Gesture::Leave(id));
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
