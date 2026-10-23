//! Hit testing. Scroll offsets shift the point before children are visited.

use super::node::{Hit, Kind};
use super::{NodeId, Tree};

impl Tree {
    pub(super) fn hit_root(&self, x: f32, y: f32) -> Option<NodeId> {
        self.root.and_then(|root| self.hit(root, x, y))
    }

    pub(super) fn hit(&self, id: NodeId, x: f32, y: f32) -> Option<NodeId> {
        let node = self.get(id)?;
        if node.w == 0.0 || node.h == 0.0 {
            return None;
        }
        if x < node.x || y < node.y || x >= node.x + node.w || y >= node.y + node.h {
            return None;
        }
        let (child_x, child_y) = if node.kind == Kind::Scroll {
            (x + node.scroll.0, y + node.scroll.1)
        } else {
            (x, y)
        };
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit(*child, child_x, child_y) {
                return Some(hit);
            }
        }
        match node.hit {
            Hit::Opaque | Hit::Region => Some(id),
            Hit::Defer => None,
        }
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

    pub(super) fn focusable_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.focusable {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    pub(super) fn region_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.hit == Hit::Region {
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
