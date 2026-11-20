//! Placement for a platform-owned view drawn above the raster surface.

use std::sync::Arc;

use super::node::{NodeFlags, Role};
use super::{NodeId, Rect, Tree};

#[derive(Clone, Debug, PartialEq)]
pub struct PlatformViewPlacement {
    pub node: NodeId,
    pub kind: Arc<str>,
    pub rect: Rect,
    pub clip: Rect,
}

impl Tree {
    pub fn set_platform_view(&mut self, node: NodeId, kind: &str) -> Result<(), crate::Error> {
        self.node(node)?;
        if kind.is_empty() {
            self.platform_views.remove(&node);
        } else if kind.len() <= 128 {
            self.platform_views.insert(node, Arc::from(kind));
        } else {
            return Err(crate::Error::BadNumber);
        }
        self.mark_paint();
        Ok(())
    }

    /// Native views follow scene paint order relative to one another.
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
        {
            return;
        }
        let transform = node.transform();
        if transform.rotate_rad != 0.0
            || transform.scale_x != 1.0
            || transform.scale_y != 1.0
            || node.alpha().is_some_and(|alpha| alpha != 255)
            || node.paints_effect()
            || ((node.is_scroll() || node.role.contains(Role::CLIP)) && node.corner_radius() > 0.0)
        {
            return;
        }
        if let Some(kind) = self.platform_views.get(&id)
            && let (Ok(rect), Ok(clip)) = (self.scene_rect(id), self.visible_scene_rect(id))
        {
            placements.push(PlatformViewPlacement {
                node: id,
                kind: Arc::clone(kind),
                rect,
                clip,
            });
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
}
