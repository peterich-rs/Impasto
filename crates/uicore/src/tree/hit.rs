//! Hit testing.
//!
//! Scroll offsets and transforms are undone so the test runs in layout space.
//! A scroll offset is subtracted from painted coordinates. A transform is the
//! canvas matrix: translate, then clockwise rotation, then uniform scale, each
//! around the current origin. Ancestor scrolls are removed first, then the
//! transforms apply from the inside out. The inverse walks transforms from the
//! outside in and adds the scroll offsets back.

use super::node::{DragClaim, Hit, Kind, TransformGeom};
use super::{NodeId, Tree};
use crate::{ScrollAccept, ScrollAxis};

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
    scene_x: f32,
    scene_y: f32,
    /// Scene point after ancestor transform inverses, applied outside-in.
    x: f32,
    y: f32,
    scroll_x: f32,
    scroll_y: f32,
    /// Maps a scroll-removed layout point through ancestor transforms to the scene.
    to_scene: Affine,
}

impl HitQuery {
    fn at_scene(x: f32, y: f32) -> Self {
        Self {
            scene_x: x,
            scene_y: y,
            x,
            y,
            scroll_x: 0.0,
            scroll_y: 0.0,
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
        let Some((kind, w, h, radius, scale_x, scale_y)) = self.get(id).map(|node| {
            let geom = node.transform();
            (
                node.kind,
                node.w,
                node.h,
                node.corner_radius(),
                geom.scale_x,
                geom.scale_y,
            )
        }) else {
            return false;
        };
        if kind == Kind::Transform {
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
        !radius_shapes_hit(kind) || rounded_contains(bx, by, w, h, radius, x, y)
    }

    /// Fills `out` from the root to the leaf under the scene point. The buffer is cleared first.
    pub fn hit_path(&self, x: f32, y: f32, out: &mut crate::HitPath) {
        out.clear();
        let (origin_x, origin_y) = self.origin;
        if let Some(root) = self.root {
            let _ = self.hit(root, HitQuery::at_scene(x, y), origin_x, origin_y, out);
        }
    }

    pub(super) fn hit_root(&self, x: f32, y: f32) -> Option<NodeId> {
        let mut path = crate::HitPath::default();
        self.hit_path(x, y, &mut path);
        path.leaf()
    }

    fn hit(
        &self,
        id: NodeId,
        query: HitQuery,
        parent_x: f32,
        parent_y: f32,
        path: &mut crate::HitPath,
    ) -> Option<NodeId> {
        let node = self.get(id)?;
        // An empty text can be stretched across a stack cell. It is not drawn
        // and must not sit in front of the scroll underneath it.
        if node.kind == Kind::Text && node.text().is_none_or(|text| text.utf8.is_empty()) {
            return None;
        }
        let abs_x = parent_x + node.x;
        let abs_y = parent_y + node.y;
        if node.w == 0.0 || node.h == 0.0 {
            return None;
        }
        if node.kind == Kind::Transform {
            let geom = node.transform();
            let (scale_x, scale_y) = (geom.scale_x, geom.scale_y);
            if scale_x.abs() < SCALE_EPSILON || scale_y.abs() < SCALE_EPSILON {
                return None;
            }
            if !transform_contains(node, abs_x, abs_y, &query) {
                return None;
            }
        } else {
            let (x, y) = query.layout_point();
            if x < abs_x || y < abs_y || x >= abs_x + node.w || y >= abs_y + node.h {
                return None;
            }
            if radius_shapes_hit(node.kind)
                && !rounded_contains(abs_x, abs_y, node.w, node.h, node.corner_radius(), x, y)
            {
                return None;
            }
        }
        let (local_x, local_y) = query.layout_point();
        path.push(crate::HitEntry {
            id,
            x: local_x - abs_x,
            y: local_y - abs_y,
        });
        let child_query = match node.kind {
            Kind::Scroll => {
                let scroll = node.scroll_paint();
                HitQuery {
                    scroll_x: query.scroll_x + scroll.0,
                    scroll_y: query.scroll_y + scroll.1,
                    ..query
                }
            }
            Kind::Transform => {
                let geom = node.transform();
                let (x, y) = inverse_transform(
                    query.x,
                    query.y,
                    geom.translate_x,
                    geom.translate_y,
                    geom.rotate_rad,
                    geom.scale_x,
                    geom.scale_y,
                )?;
                HitQuery {
                    x,
                    y,
                    to_scene: query.to_scene.around(Affine::paint(
                        geom.translate_x,
                        geom.translate_y,
                        geom.rotate_rad,
                        geom.scale_x,
                        geom.scale_y,
                    )),
                    ..query
                }
            }
            _ => query,
        };
        let mut child = node.last_child;
        let role = node.hit_role();
        let kind = node.kind;
        let _ = node;
        while let Some(current) = child {
            let prev = self.get(current)?.prev;
            if let Some(hit) = self.hit(current, child_query, abs_x, abs_y, path) {
                return Some(hit);
            }
            child = prev;
        }
        match role {
            Hit::Opaque | Hit::Region => Some(id),
            // The viewport is the drag target. A point in the box with no child
            // still starts a scroll, and does not become a press: the role stays
            // defer, so `region_of` walks through it.
            Hit::Defer if kind == Kind::Scroll => Some(id),
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
        if node.kind == Kind::Transform {
            transforms.insert(0, node.transform());
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

    fn ancestor_paint(&self, id: NodeId) -> Option<AncestorPaint> {
        let mut scroll_x = 0.0;
        let mut scroll_y = 0.0;
        let mut transforms = Vec::new();
        let mut parent = self.get(id)?.parent;
        while let Some(pid) = parent {
            let node = self.get(pid)?;
            if node.kind == Kind::Scroll {
                let scroll = node.scroll_paint();
                scroll_x += scroll.0;
                scroll_y += scroll.1;
            }
            if node.kind == Kind::Transform {
                transforms.push(node.transform());
            }
            parent = node.parent;
        }
        Some(AncestorPaint {
            scroll_x,
            scroll_y,
            transforms,
        })
    }

    fn transformed_box_contains(&self, id: NodeId, scene_x: f32, scene_y: f32) -> bool {
        let Some(node) = self.get(id) else {
            return false;
        };
        let (w, h) = (node.w, node.h);
        let _ = node;
        let Ok((bx, by)) = self.layout_point(id) else {
            return false;
        };
        let corners = [(bx, by), (bx + w, by), (bx + w, by + h), (bx, by + h)];
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for (x, y) in corners {
            let Some((sx, sy)) = self.layout_to_scene(id, x, y) else {
                return false;
            };
            min_x = min_x.min(sx);
            min_y = min_y.min(sy);
            max_x = max_x.max(sx);
            max_y = max_y.max(sy);
        }
        scene_x >= min_x && scene_y >= min_y && scene_x < max_x && scene_y < max_y
    }

    pub(super) fn drag_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.kind == Kind::Scroll {
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
        pointer: Option<crate::PointerId>,
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
            if node.kind == Kind::Scroll && !self.scroll_held_by_other(id, pointer) {
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

    fn scroll_held_by_other(&self, id: NodeId, pointer: Option<crate::PointerId>) -> bool {
        crate::ScrollRecognizer::locked_by_other(&self.arena.scroll_locks, id, pointer)
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

fn transform_contains(node: &super::node::Node, abs_x: f32, abs_y: f32, query: &HitQuery) -> bool {
    let geom = node.transform();
    let map = query.to_scene.around(Affine::paint(
        geom.translate_x,
        geom.translate_y,
        geom.rotate_rad,
        geom.scale_x,
        geom.scale_y,
    ));
    let corners = [
        (abs_x, abs_y),
        (abs_x + node.w, abs_y),
        (abs_x + node.w, abs_y + node.h),
        (abs_x, abs_y + node.h),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (x, y) in corners {
        let (sx, sy) = map.apply(x - query.scroll_x, y - query.scroll_y);
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    query.scene_x >= min_x
        && query.scene_y >= min_y
        && query.scene_x < max_x
        && query.scene_y < max_y
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

fn radius_shapes_hit(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Clip | Kind::Text | Kind::Image | Kind::PointerRegion
    )
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
