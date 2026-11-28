//! Hit testing.
//!
//! Scroll offsets and transforms are undone so the test runs in layout space.
//! A scroll offset is subtracted from painted coordinates. A transform is the
//! canvas matrix: translate, then clockwise rotation, then uniform scale, each
//! around the current origin. Ancestor scrolls are removed first, then the
//! transforms apply from the inside out. The inverse walks transforms from the
//! outside in and adds the scroll offsets back.

use super::node::{DragClaim, Hit, TransformGeom};
use super::{NodeId, Tree};
use crate::motion::{ScrollAccept, ScrollAxis};

const SCALE_EPSILON: f32 = 1.0e-4;

/// Ancestor scroll sum, and transforms from the inside out.
struct AncestorPaint {
    scroll_x: f32,
    scroll_y: f32,
    transforms: Vec<TransformGeom>,
}

/// Point carried down the tree. Scroll sums and the ancestor transform matrix
/// update as the walk enters those nodes, so a node does not revisit its parents.
#[derive(Clone, Copy)]
struct HitQuery {
    /// Scene point after ancestor transform inverses, applied outside-in.
    x: f32,
    y: f32,
    scroll_x: f32,
    scroll_y: f32,
    cursor: bool,
    /// Maps a scroll-removed layout point through ancestor transforms to the scene.
    to_scene: Affine,
}

impl HitQuery {
    fn at_scene(x: f32, y: f32) -> Self {
        Self {
            x,
            y,
            scroll_x: 0.0,
            scroll_y: 0.0,
            cursor: false,
            to_scene: Affine::identity(),
        }
    }

    fn layout_point(&self) -> (f32, f32) {
        (self.x + self.scroll_x, self.y + self.scroll_y)
    }
}

impl Tree {
    /// True when the scene point lies in `id`'s own box, including a transform's
    /// scene bounds. Does not walk the rest of the tree.
    pub(super) fn point_hits_node(&self, id: NodeId, scene_x: f32, scene_y: f32) -> bool {
        let Some((transformed, shaped, w, h, radius, corners, scale_x, scale_y)) =
            self.get(id).map(|node| {
                let geom = self
                    .effective_transform(id)
                    .unwrap_or_else(|| node.transform());
                (
                    !geom.is_identity(),
                    radius_shapes_hit(node),
                    node.w,
                    node.h,
                    node.corner_radius(),
                    node.corners(),
                    geom.scale_x,
                    geom.scale_y,
                )
            })
        else {
            return false;
        };
        if transformed {
            return scale_x.abs() >= SCALE_EPSILON
                && scale_y.abs() >= SCALE_EPSILON
                && self.transformed_box_contains(id, scene_x, scene_y);
        }
        let Some((x, y)) = self.scene_to_layout(id, scene_x, scene_y) else {
            return false;
        };
        let Ok((bx, by)) = self.layout_point(id) else {
            return false;
        };
        if x < bx || y < by || x >= bx + w || y >= by + h {
            return false;
        }
        !shaped
            || corners.map_or_else(
                || rounded_contains(bx, by, w, h, radius, x, y),
                |shape| shape.contains([w, h], [x - bx, y - by]),
            )
    }

    /// Fills `out` from the root to the leaf under the scene point. The buffer is cleared first.
    pub fn hit_path(&self, x: f32, y: f32, out: &mut crate::input::HitPath) {
        out.clear();
        let (origin_x, origin_y) = self.origin;
        if let Some(root) = self.root {
            let _ = self.hit(root, HitQuery::at_scene(x, y), origin_x, origin_y, out);
        }
    }

    /// Returns a new cursor only when the pointer's resolved shape changes.
    pub fn take_cursor(&mut self) -> Option<crate::style::CursorStyle> {
        use crate::style::CursorStyle;

        let mut next = CursorStyle::Default;
        if !self.cursors.is_empty()
            && let (Some(root), Some((x, y))) = (self.root, self.cursor_pointer)
        {
            let mut path = std::mem::take(&mut self.cursor_hit);
            path.clear();
            let mut query = HitQuery::at_scene(x, y);
            query.cursor = true;
            self.hit(root, query, self.origin.0, self.origin.1, &mut path);
            next = path
                .entries()
                .iter()
                .rev()
                .find_map(|entry| self.cursors.get(&entry.id).copied())
                .unwrap_or_default();
            self.cursor_hit = path;
        }
        if next == self.cursor_sent {
            return None;
        }
        self.cursor_sent = next;
        Some(next)
    }

    pub(super) fn hit_root(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut path = crate::input::HitPath::default();
        self.hit_path(x, y, &mut path);
        path.leaf()
    }

    fn hit(
        &self,
        id: NodeId,
        query: HitQuery,
        parent_x: f32,
        parent_y: f32,
        path: &mut crate::input::HitPath,
    ) -> Option<NodeId> {
        let node = self.get(id)?;
        if node.flags.contains(super::node::NodeFlags::CULL_PAINT)
            || node.flags.contains(super::node::NodeFlags::HIDDEN)
        {
            return None;
        }
        // An empty text can be stretched across a stack cell. It is not drawn
        // and must not sit in front of the scroll underneath it.
        if node.is_text() && node.text().is_none_or(|text| text.utf8.is_empty()) {
            return None;
        }
        let abs_x = parent_x + node.x;
        let abs_y = parent_y + node.y;
        let mut child_query = query;
        if let Some(geom) = self
            .effective_transform(id)
            .filter(|geom| !geom.is_identity())
        {
            let geom = geom.placed(abs_x, abs_y, node.w, node.h);
            if geom.scale_x.abs() < SCALE_EPSILON || geom.scale_y.abs() < SCALE_EPSILON {
                return None;
            }
            let (x, y) = inverse_transform(
                query.x,
                query.y,
                geom.translate_x,
                geom.translate_y,
                geom.rotate_rad,
                geom.scale_x,
                geom.scale_y,
            )?;
            child_query.x = x;
            child_query.y = y;
            child_query.to_scene = query.to_scene.around(Affine::paint(
                geom.translate_x,
                geom.translate_y,
                geom.rotate_rad,
                geom.scale_x,
                geom.scale_y,
            ));
        }
        let (x, y) = child_query.layout_point();
        let inside = node.w > 0.0
            && node.h > 0.0
            && x >= abs_x
            && y >= abs_y
            && x < abs_x + node.w
            && y < abs_y + node.h
            && (!radius_shapes_hit(node)
                || node.corners().map_or_else(
                    || rounded_contains(abs_x, abs_y, node.w, node.h, node.corner_radius(), x, y),
                    |shape| shape.contains([node.w, node.h], [x - abs_x, y - abs_y]),
                ));
        if !inside && (node.is_scroll() || node.role.contains(super::node::Role::CLIP)) {
            return None;
        }
        path.push(crate::input::HitEntry {
            id,
            x: x - abs_x,
            y: y - abs_y,
        });
        if node.is_scroll() {
            let scroll = node.scroll_paint();
            child_query.scroll_x += scroll.0;
            child_query.scroll_y += scroll.1;
        }
        let role = node.hit_role();
        let kind = node.kind;
        let fallback = node.last_child;
        let _ = node;
        if let Some(list) = self.ordered_children(id, true) {
            for current in list {
                if let Some(hit) = self.hit(current, child_query, abs_x, abs_y, path) {
                    return Some(hit);
                }
            }
        } else {
            let mut child = fallback;
            while let Some(current) = child {
                let prev = self.get(current)?.prev;
                if let Some(hit) = self.hit(current, child_query, abs_x, abs_y, path) {
                    return Some(hit);
                }
                child = prev;
            }
        }
        if !inside {
            path.pop();
            return None;
        }
        if query.cursor && self.cursors.contains_key(&id) {
            return Some(id);
        }
        match role {
            Hit::Opaque | Hit::Region => Some(id),
            // The viewport is the drag target. A point in the box with no child
            // still starts a scroll, and does not become a press: the role stays
            // defer, so `region_of` walks through it.
            Hit::Defer if kind == super::node::Kind::Scroll => Some(id),
            Hit::Defer => {
                path.pop();
                None
            }
        }
    }

    /// Layout-space point for a scene point, undoing ancestor scrolls and transforms.
    pub(super) fn scene_to_layout(&self, id: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let AncestorPaint {
            scroll_x,
            scroll_y,
            mut transforms,
        } = self.ancestor_paint(id)?;
        let mut x = x;
        let mut y = y;
        while let Some(geom) = transforms.pop() {
            if geom.scale_x.abs() < SCALE_EPSILON || geom.scale_y.abs() < SCALE_EPSILON {
                return None;
            }
            (x, y) = inverse_transform(
                x,
                y,
                geom.translate_x,
                geom.translate_y,
                geom.rotate_rad,
                geom.scale_x,
                geom.scale_y,
            )?;
        }
        Some((x + scroll_x, y + scroll_y))
    }

    /// Scene point for a layout point on `id`, including `id`'s own transform.
    pub(super) fn layout_to_scene(&self, id: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        let node = self.get(id)?;
        let AncestorPaint {
            scroll_x,
            scroll_y,
            mut transforms,
        } = self.ancestor_paint(id)?;
        if let Some(geom) = self
            .effective_transform(id)
            .filter(|geom| !geom.is_identity())
        {
            let (px, py) = self.layout_point(id).unwrap_or((0.0, 0.0));
            transforms.insert(0, geom.placed(px, py, node.w, node.h));
        }
        let mut x = x - scroll_x;
        let mut y = y - scroll_y;
        for geom in transforms {
            (x, y) = forward_transform(
                x,
                y,
                geom.translate_x,
                geom.translate_y,
                geom.rotate_rad,
                geom.scale_x,
                geom.scale_y,
            );
        }
        Some((x, y))
    }

    /// Canvas content-box point for a scene point.
    ///
    /// Undoes this node's transform, then ancestor transforms outside-in, then
    /// ancestor scroll. Does not undo the canvas CTM. `None` when `id` is
    /// missing, hidden, or a transform on that path is singular.
    pub fn canvas_local(&self, id: NodeId, scene_x: f32, scene_y: f32) -> Option<(f32, f32)> {
        let node = self.get(id)?;
        if node.flags.contains(super::node::NodeFlags::HIDDEN) {
            return None;
        }
        let content_x = node.content_x;
        let content_y = node.content_y;
        let AncestorPaint {
            scroll_x,
            scroll_y,
            mut transforms,
        } = self.ancestor_paint(id)?;
        if let Some(geom) = self
            .effective_transform(id)
            .filter(|geom| !geom.is_identity())
        {
            let (px, py) = self.layout_point(id).ok()?;
            transforms.insert(0, geom.placed(px, py, node.w, node.h));
        }
        let mut x = scene_x;
        let mut y = scene_y;
        while let Some(geom) = transforms.pop() {
            (x, y) = inverse_transform(
                x,
                y,
                geom.translate_x,
                geom.translate_y,
                geom.rotate_rad,
                geom.scale_x,
                geom.scale_y,
            )?;
        }
        let (origin_x, origin_y) = self.layout_point(id).ok()?;
        Some((
            x + scroll_x - origin_x - content_x,
            y + scroll_y - origin_y - content_y,
        ))
    }

    fn ancestor_paint(&self, id: NodeId) -> Option<AncestorPaint> {
        let mut scroll_x = 0.0;
        let mut scroll_y = 0.0;
        let mut transforms = Vec::new();
        let mut parent = self.get(id)?.parent;
        while let Some(pid) = parent {
            let node = self.get(pid)?;
            if node.is_scroll() {
                let scroll = node.scroll_paint();
                scroll_x += scroll.0;
                scroll_y += scroll.1;
            }
            if let Some(geom) = self
                .effective_transform(pid)
                .filter(|geom| !geom.is_identity())
            {
                let (px, py) = self.layout_point(pid).unwrap_or((0.0, 0.0));
                transforms.push(geom.placed(px, py, node.w, node.h));
            }
            parent = node.parent;
        }
        Some(AncestorPaint {
            scroll_x,
            scroll_y,
            transforms,
        })
    }

    /// Visual scene box. Layout box plus ancestor scroll and transform.
    ///
    /// [`Tree::rect`] is the layout box and must not be used to place a panel
    /// or a caret. A rotated node returns the axis-aligned bounds of its corners.
    pub fn scene_rect(&self, id: NodeId) -> Result<super::Rect, crate::Error> {
        let node = self.node(id)?;
        let (bx, by) = self.layout_point(id)?;
        let (w, h) = (node.w, node.h);
        let corners = [(bx, by), (bx + w, by), (bx + w, by + h), (bx, by + h)];
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for (x, y) in corners {
            let Some((sx, sy)) = self.layout_to_scene(id, x, y) else {
                return self.rect(id);
            };
            min_x = min_x.min(sx);
            min_y = min_y.min(sy);
            max_x = max_x.max(sx);
            max_y = max_y.max(sy);
        }
        Ok(super::Rect {
            x: min_x,
            y: min_y,
            w: (max_x - min_x).max(0.0),
            h: (max_y - min_y).max(0.0),
        })
    }

    /// The visible part of a projected scene box after ancestor viewport clips.
    pub(super) fn visible_scene_rect(&self, id: NodeId) -> Result<super::Rect, crate::Error> {
        let mut rect = self.scene_rect(id)?;
        let mut current = self.node(id)?.parent;
        while let Some(ancestor) = current {
            let node = self.node(ancestor)?;
            if node.is_scroll() || node.role.contains(super::node::Role::CLIP) {
                let clip = self.scene_rect(ancestor)?;
                let right = (rect.x + rect.w).min(clip.x + clip.w);
                let bottom = (rect.y + rect.h).min(clip.y + clip.h);
                rect.x = rect.x.max(clip.x);
                rect.y = rect.y.max(clip.y);
                rect.w = (right - rect.x).max(0.0);
                rect.h = (bottom - rect.y).max(0.0);
            }
            current = node.parent;
        }
        Ok(rect)
    }

    fn transformed_box_contains(&self, id: NodeId, scene_x: f32, scene_y: f32) -> bool {
        let Ok(box_) = self.scene_rect(id) else {
            return false;
        };
        scene_x >= box_.x
            && scene_y >= box_.y
            && scene_x < box_.x + box_.w
            && scene_y < box_.y + box_.h
    }

    pub(super) fn drag_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.is_scroll() {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    /// Scroll that should take a drag or a wheel sample.
    ///
    /// A declared axis looks only at its own component. `Both` still picks the
    /// larger delta. `Never` is skipped. `Always` claims that axis even at the
    /// edge. `Content` needs room in the drag direction. When nothing can move,
    /// a drag still locks the innermost `Both` scroll that overflows, so the
    /// gesture does not become a click. A declared axis does not use that
    /// fallback: edge capture is `ScrollAccept::Always`. A wheel does not lock.
    pub(super) fn scroll_for(
        &self,
        start: NodeId,
        dx: f32,
        dy: f32,
        pointer: Option<crate::input::PointerId>,
        lock_at_edge: bool,
    ) -> Option<NodeId> {
        if dx == 0.0 && dy == 0.0 {
            return None;
        }
        let horizontal = dx.abs() >= dy.abs();
        let mut id = start;
        let mut edge: Option<NodeId> = None;
        loop {
            let node = self.get(id)?;
            if node.is_scroll() && !self.scroll_held_by_other(id, pointer) {
                let (axis, accept) = node
                    .scroll_geom()
                    .map(|geom| (geom.axis, geom.physics.accept))
                    .unwrap_or((ScrollAxis::Both, ScrollAccept::Content));
                if accept != ScrollAccept::Never {
                    if self.can_consume(id, axis, accept, dx, dy) {
                        return Some(id);
                    }
                    if edge.is_none() && axis == ScrollAxis::Both && self.overflows(id, horizontal)
                    {
                        edge = Some(id);
                    }
                }
            }
            let Some(parent) = node.parent else {
                break;
            };
            id = parent;
        }
        lock_at_edge.then_some(edge).flatten()
    }

    fn scroll_held_by_other(&self, id: NodeId, pointer: Option<crate::input::PointerId>) -> bool {
        crate::input::gesture::scroll_locked_by_other(&self.arena.scroll_locks, id, pointer)
    }

    fn overflows(&self, id: NodeId, horizontal: bool) -> bool {
        let Some(node) = self.get(id) else {
            return false;
        };
        let Some((content_w, content_h)) = self.scroll_content(id) else {
            return false;
        };
        if horizontal {
            content_w > node.w + 0.5
        } else {
            content_h > node.h + 0.5
        }
    }

    /// `dx` and `dy` use drag sign: positive moves the content with the finger.
    fn can_consume(
        &self,
        id: NodeId,
        axis: ScrollAxis,
        accept: ScrollAccept,
        dx: f32,
        dy: f32,
    ) -> bool {
        match accept {
            ScrollAccept::Never => false,
            ScrollAccept::Always => match axis {
                ScrollAxis::Vertical => dy != 0.0,
                ScrollAxis::Horizontal => dx != 0.0,
                ScrollAxis::Both => true,
            },
            ScrollAccept::Content => match axis {
                ScrollAxis::Vertical => self.room_along(id, false, dy),
                ScrollAxis::Horizontal => self.room_along(id, true, dx),
                ScrollAxis::Both => {
                    if dx.abs() >= dy.abs() {
                        self.room_along(id, true, dx)
                    } else {
                        self.room_along(id, false, dy)
                    }
                }
            },
        }
    }

    fn room_along(&self, id: NodeId, horizontal: bool, delta: f32) -> bool {
        if delta == 0.0 {
            return false;
        }
        let Some(node) = self.get(id) else {
            return false;
        };
        let Some((content_w, content_h)) = self.scroll_content(id) else {
            return false;
        };
        let (offset_x, offset_y) = node.scroll_offset();
        if horizontal {
            if content_w <= node.w + 0.5 {
                return false;
            }
            if delta > 0.0 {
                offset_x > 0.5
            } else {
                offset_x + node.w < content_w - 0.5
            }
        } else if content_h <= node.h + 0.5 {
            false
        } else if delta > 0.0 {
            offset_y > 0.5
        } else {
            offset_y + node.h < content_h - 0.5
        }
    }

    pub(super) fn scroll_content(&self, id: NodeId) -> Option<(f32, f32)> {
        let node = self.get(id)?;
        let mut width: f32 = 0.0;
        let mut height: f32 = 0.0;
        let mut child = node.first_child;
        while let Some(current) = child {
            let next = self.get(current)?.next;
            let child_node = self.get(current)?;
            width = width.max(child_node.w);
            height = height.max(child_node.h);
            child = next;
        }
        Some((width, height))
    }

    pub(super) fn claims_drag(&self, id: NodeId, dx: f32, dy: f32) -> bool {
        let Some(node) = self.get(id) else {
            return false;
        };
        if let Some(axis) = node.recognizers.drag {
            return axis.matches(dx, dy);
        }
        match node.drag_claim {
            DragClaim::None => false,
            DragClaim::Free => dx != 0.0 || dy != 0.0,
            DragClaim::Horizontal => dx.abs() >= dy.abs() && dx != 0.0,
        }
    }

    pub(super) fn focusable_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            if !self.inside_scope(id) {
                return None;
            }
            let node = self.get(id)?;
            if node.is_focusable() {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    pub(super) fn inside_scope(&self, id: NodeId) -> bool {
        let Some(scope) = self.scopes.last().copied() else {
            return true;
        };
        id == scope || self.contains(scope, id)
    }

    pub(super) fn region_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.hit_role() == Hit::Region {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    pub(super) fn contains(&self, target: NodeId, mut hit: NodeId) -> bool {
        loop {
            if hit == target {
                return true;
            }
            let Some(node) = self.get(hit) else {
                return false;
            };
            let Some(parent) = node.parent else {
                return false;
            };
            hit = parent;
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    pub(crate) fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 1.0,
            f: 0.0,
        }
    }

    #[cfg(test)]
    pub(crate) fn translate(x: f32, y: f32) -> Self {
        Self::paint(x, y, 0.0, 1.0, 1.0)
    }

    pub(crate) fn paint(
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    ) -> Self {
        let (sin, cos) = rotate_rad.sin_cos();
        Self {
            a: scale_x * cos,
            b: -scale_y * sin,
            c: translate_x,
            d: scale_x * sin,
            e: scale_y * cos,
            f: translate_y,
        }
    }

    #[cfg(test)]
    pub(crate) fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        )
    }

    /// `self` applied after `inner`.
    pub(crate) fn around(self, inner: Self) -> Self {
        Self {
            a: self.a * inner.a + self.b * inner.d,
            b: self.a * inner.b + self.b * inner.e,
            c: self.a * inner.c + self.b * inner.f + self.c,
            d: self.d * inner.a + self.e * inner.d,
            e: self.d * inner.b + self.e * inner.e,
            f: self.d * inner.c + self.e * inner.f + self.f,
        }
    }
}

fn radius_shapes_hit(node: &super::node::Node) -> bool {
    node.role.contains(super::node::Role::CLIP)
        || node.is_text()
        || node.is_image()
        || node.is_pointer()
}

fn rounded_contains(x: f32, y: f32, w: f32, h: f32, radius: f32, px: f32, py: f32) -> bool {
    let radius = radius.max(0.0).min(w.min(h) * 0.5);
    if radius <= 0.0 {
        return true;
    }
    let left = x;
    let top = y;
    let right = x + w;
    let bottom = y + h;
    if px >= left + radius && px < right - radius {
        return py >= top && py < bottom;
    }
    if py >= top + radius && py < bottom - radius {
        return px >= left && px < right;
    }
    let cx = if px < left + radius {
        left + radius
    } else {
        right - radius
    };
    let cy = if py < top + radius {
        top + radius
    } else {
        bottom - radius
    };
    let dx = px - cx;
    let dy = py - cy;
    dx * dx + dy * dy <= radius * radius
}

/// Scale, then clockwise rotation, then translation. Y grows downward.
pub(super) fn forward_transform(
    x: f32,
    y: f32,
    translate_x: f32,
    translate_y: f32,
    rotate_rad: f32,
    scale_x: f32,
    scale_y: f32,
) -> (f32, f32) {
    let (sin, cos) = rotate_rad.sin_cos();
    let x = x * scale_x;
    let y = y * scale_y;
    (
        x * cos - y * sin + translate_x,
        x * sin + y * cos + translate_y,
    )
}

pub(super) fn inverse_transform(
    x: f32,
    y: f32,
    translate_x: f32,
    translate_y: f32,
    rotate_rad: f32,
    scale_x: f32,
    scale_y: f32,
) -> Option<(f32, f32)> {
    if scale_x.abs() < SCALE_EPSILON || scale_y.abs() < SCALE_EPSILON {
        return None;
    }
    let x = x - translate_x;
    let y = y - translate_y;
    let (sin, cos) = rotate_rad.sin_cos();
    let unrotated_x = x * cos + y * sin;
    let unrotated_y = -x * sin + y * cos;
    Some((unrotated_x / scale_x, unrotated_y / scale_y))
}
