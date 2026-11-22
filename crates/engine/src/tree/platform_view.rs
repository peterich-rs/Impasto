//! Paint-ordered placement for a platform-owned view.

use std::sync::Arc;

use crate::scene::PathVerb;
use kurbo::BezPath;

use super::node::{NodeFlags, Role};
use super::{NodeId, Rect, Tree};

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformViewPlacement {
    pub node: NodeId,
    pub kind: Arc<str>,
    pub rect: Rect,
    pub clip: Rect,
    pub geometry: PlatformViewGeometry,
    pub occluders: Vec<Rect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformViewGeometry {
    pub width: f32,
    pub height: f32,
    /// Local points to scene points. Order matches CGAffineTransform.
    pub transform: [f32; 6],
    pub opacity: f32,
    pub clips: Vec<PlatformViewClip>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformViewClip {
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub transform: [f32; 6],
    pub path: Option<Arc<[PathVerb]>>,
}

#[derive(PartialEq)]
pub(super) struct PlatformViewSpec {
    kind: Arc<str>,
    path: Option<Arc<[PathVerb]>>,
}

impl PlatformViewPlacement {
    pub fn supports_rectangular_overlay(&self) -> bool {
        let axis_aligned = |transform: &[f32; 6]| transform[..4] == [1.0, 0.0, 0.0, 1.0];
        self.geometry.opacity == 1.0
            && axis_aligned(&self.geometry.transform)
            && self.geometry.clips.iter().all(|clip| {
                clip.path.is_none() && clip.radius == 0.0 && axis_aligned(&clip.transform)
            })
    }
}

impl Tree {
    pub fn set_platform_view(&mut self, node: NodeId, kind: &str) -> Result<(), crate::Error> {
        self.set_platform_view_with_path(node, kind, None)
    }

    /// The optional SVG path clips a native slot in its local logical points.
    pub fn set_platform_view_with_path(
        &mut self,
        node: NodeId,
        kind: &str,
        svg: Option<&str>,
    ) -> Result<(), crate::Error> {
        self.node(node)?;
        let changed = if kind.is_empty() {
            self.platform_views.remove(&node).is_some()
        } else if kind.len() <= 128 {
            let path = svg.map(parse_path).transpose()?;
            let spec = PlatformViewSpec {
                kind: Arc::from(kind),
                path,
            };
            if self.platform_views.get(&node) == Some(&spec) {
                return Ok(());
            }
            self.platform_views.insert(node, spec);
            true
        } else {
            return Err(crate::Error::BadNumber);
        };
        if changed {
            self.mark_paint();
            self.mark_semantics_chain(node);
        }
        Ok(())
    }

    pub fn platform_views(&self) -> Vec<PlatformViewPlacement> {
        if self.platform_views.is_empty() || !self.laid_out {
            return Vec::new();
        }
        let mut placements = Vec::with_capacity(self.platform_views.len());
        if let Some(root) = self.root {
            self.collect_platform_views(root, &mut placements);
        }
        placements
    }

    fn collect_platform_views(&self, id: NodeId, placements: &mut Vec<PlatformViewPlacement>) {
        let Some(node) = self.get(id) else { return };
        if node
            .flags
            .intersects(NodeFlags::CULL_PAINT | NodeFlags::HIDDEN | NodeFlags::SEMANTICS_HIDDEN)
            || node.alpha() == Some(0)
        {
            return;
        }
        if let Some(placement) = self.platform_view_placement(id) {
            placements.push(placement);
        } else if node.is_pointer()
            && !placements.is_empty()
            && let Ok(box_) = self.visible_scene_rect(id)
        {
            for view in &mut *placements {
                if let Some(overlap) = intersection(view.clip, box_) {
                    view.occluders.push(overlap);
                }
            }
        }
        if let Some(ordered) = self.ordered_children(id, false) {
            for child in ordered {
                self.collect_platform_views(child, placements);
            }
        } else {
            let mut child = node.first_child;
            while let Some(id) = child {
                self.collect_platform_views(id, placements);
                child = self.get(id).and_then(|node| node.next);
            }
        }
    }

    fn platform_view_placement(&self, id: NodeId) -> Option<PlatformViewPlacement> {
        let spec = self.platform_views.get(&id)?;
        let rect = self.scene_rect(id).ok()?;
        let clip = self.visible_scene_rect(id).ok()?;
        if clip.w <= 0.0 || clip.h <= 0.0 {
            return None;
        }
        let node = self.get(id)?;
        let mut geometry = PlatformViewGeometry {
            width: node.w,
            height: node.h,
            transform: self.platform_transform(id)?,
            opacity: 1.0,
            clips: Vec::new(),
        };
        if let Some(path) = &spec.path {
            geometry.clips.push(PlatformViewClip {
                width: node.w,
                height: node.h,
                radius: 0.0,
                transform: geometry.transform,
                path: Some(Arc::clone(path)),
            });
        }
        let mut next = Some(id);
        while let Some(current) = next {
            let node = self.get(current)?;
            // Filters blending UIKit's contents with the raster backdrop need
            // a producer texture. A view still supports transforms and masks.
            if node
                .flags
                .intersects(NodeFlags::CULL_PAINT | NodeFlags::HIDDEN | NodeFlags::SEMANTICS_HIDDEN)
                || node.alpha() == Some(0)
                || node.blur() > 0.0
                || node.backdrop() > 0.0
                || node.blend() != crate::scene::BlendMode::SrcOver
            {
                return None;
            }
            geometry.opacity *= f32::from(node.alpha().unwrap_or(255)) / 255.0;
            if node.is_scroll() || node.role.contains(Role::CLIP) {
                geometry.clips.push(PlatformViewClip {
                    width: node.w,
                    height: node.h,
                    radius: node.corner_radius(),
                    transform: self.platform_transform(current)?,
                    path: None,
                });
            }
            next = node.parent;
        }
        Some(PlatformViewPlacement {
            node: id,
            kind: Arc::clone(&spec.kind),
            rect,
            clip,
            geometry,
            occluders: Vec::new(),
        })
    }

    pub(super) fn records_platform_view(&self, id: NodeId) -> bool {
        self.platform_view_placement(id).is_some()
    }

    fn platform_transform(&self, id: NodeId) -> Option<[f32; 6]> {
        let (x, y) = self.layout_point(id).ok()?;
        let node = self.get(id)?;
        let w = node.w.max(1.0);
        let h = node.h.max(1.0);
        let (tx, ty) = self.layout_to_scene(id, x, y)?;
        let (ax, ay) = self.layout_to_scene(id, x + w, y)?;
        let (bx, by) = self.layout_to_scene(id, x, y + h)?;
        let a = (ax - tx) / w;
        let b = (ay - ty) / w;
        let c = (bx - tx) / h;
        let d = (by - ty) / h;
        let matrix = [a, b, c, d, tx, ty];
        (matrix.iter().all(|v| v.is_finite()) && (a * d - b * c).abs() > f32::EPSILON)
            .then_some(matrix)
    }
}

fn parse_path(svg: &str) -> Result<Arc<[PathVerb]>, crate::Error> {
    if svg.len() > 16 * 1024 {
        return Err(crate::Error::BadNumber);
    }
    let path = BezPath::from_svg(svg).map_err(|_| crate::Error::BadNumber)?;
    if path.elements().is_empty() || path.elements().len() > 256 {
        return Err(crate::Error::BadNumber);
    }
    path.elements()
        .iter()
        .copied()
        .map(crate::canvas::path_verb)
        .collect::<Option<_>>()
        .ok_or(crate::Error::BadNumber)
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let w = (a.x + a.w).min(b.x + b.w) - x;
    let h = (a.y + a.h).min(b.y + b.h) - y;
    (w > 0.0 && h > 0.0).then_some(Rect { x, y, w, h })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_native_slot_retains_its_path_if_a_replacement_is_invalid() -> Result<(), crate::Error> {
        let mut tree = Tree::new(crate::text::MonoShaper::new());
        let view = tree.column(0.0)?;
        tree.set_platform_view_with_path(view, "preview", Some("M0 0 L30 0 L30 30 Z"))?;
        assert!(
            tree.set_platform_view_with_path(view, "preview", Some("M1e1000 0"))
                .is_err()
        );
        assert_eq!(
            tree.platform_views
                .get(&view)
                .and_then(|spec| spec.path.as_deref()),
            Some(
                [
                    PathVerb::Move(0.0, 0.0),
                    PathVerb::Line(30.0, 0.0),
                    PathVerb::Line(30.0, 30.0),
                    PathVerb::Close
                ]
                .as_slice()
            ),
        );
        tree.set_platform_view(view, "preview")?;
        assert!(
            tree.platform_views
                .get(&view)
                .is_some_and(|spec| spec.path.is_none())
        );
        Ok(())
    }
}
