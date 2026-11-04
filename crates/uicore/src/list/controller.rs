//! Id map for one scroll. Clamp, stretch, and pull live on the scroll node.

use crate::PullMode;
use crate::tree::{NodeId, Tree};

use super::window::REFRESH_THRESHOLD;

/// Leftover from the old drag step. Pending is always false; the tree already wrote.
#[derive(Clone, Copy, Debug)]
pub struct DragStep {
    pub overflow_x: f32,
    pub overflow_y: f32,
    pub over_x: f32,
    pub over_y: f32,
    pub pending: bool,
}

/// Kept so a script can still name a parent driver. Overflow no longer walks it.
#[derive(Clone, Copy, Debug)]
pub struct ParentLink {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Debug)]
pub struct ScrollController {
    viewport: Option<NodeId>,
    content: Option<NodeId>,
    content_min: f32,
    limits_x: f32,
    limits_y: f32,
    off_x: f32,
    off_y: f32,
    threshold: f32,
    pub parent: Option<ParentLink>,
    axis: crate::ScrollAxis,
}

impl Default for ScrollController {
    fn default() -> Self {
        Self {
            viewport: None,
            content: None,
            content_min: 0.0,
            limits_x: 0.0,
            limits_y: 0.0,
            off_x: 0.0,
            off_y: 0.0,
            threshold: REFRESH_THRESHOLD,
            parent: None,
            axis: crate::ScrollAxis::Both,
        }
    }
}

impl ScrollController {
    pub fn bind(&mut self, viewport: Option<NodeId>, content: Option<NodeId>, tree: &Tree) {
        if let Some(id) = viewport {
            self.viewport = Some(id);
        }
        if let Some(id) = content {
            self.content = Some(id);
        }
        if let Some(id) = self.viewport
            && let Ok((x, y)) = tree.scroll_offset(id)
        {
            self.off_x = x;
            self.off_y = y;
        }
        self.refresh_boxes(tree);
    }

    pub fn configure(
        &mut self,
        content_min: f32,
        _pull_mode: PullMode,
        threshold: f32,
        _rubber: bool,
        _factor: f32,
        parent: Option<ParentLink>,
    ) {
        if content_min.is_finite() && content_min >= 0.0 {
            self.content_min = content_min;
        }
        if threshold.is_finite() && threshold >= 0.0 {
            self.threshold = threshold;
        }
        self.parent = parent;
    }

    pub fn viewport(&self) -> Option<NodeId> {
        self.viewport
    }

    pub fn limits(&self) -> (f32, f32) {
        (self.limits_x, self.limits_y)
    }

    pub fn refresh_boxes(&mut self, tree: &Tree) {
        let Some(viewport) = self.viewport else {
            return;
        };
        let mut view_h = 0.0;
        let mut body_w = 0.0;
        let mut body_h = 0.0;
        if let Ok(view) = tree.rect(viewport) {
            view_h = view.h;
            self.limits_x = 0.0;
        }
        if let Some(content) = self.content
            && let Ok(body) = tree.rect(content)
        {
            body_w = body.w;
            body_h = body.h;
        }
        if let Ok(view) = tree.rect(viewport) {
            self.limits_x = (body_w - view.w).max(0.0);
        }
        self.limits_y = (body_h - view_h)
            .max(0.0)
            .max(self.content_min - view_h)
            .max(0.0);
        if let Ok(axis) = tree.scroll_axis(viewport) {
            self.axis = axis;
        }
        let (x, y) = self.axis.filter(self.limits_x, self.limits_y);
        self.limits_x = x;
        self.limits_y = y;
    }

    pub fn drag_by(&mut self, tree: &Tree, _dx: f32, _dy: f32) -> DragStep {
        let (over_x, over_y) = self
            .viewport
            .and_then(|id| tree.overscroll(id).ok())
            .unwrap_or((0.0, 0.0));
        DragStep {
            overflow_x: 0.0,
            overflow_y: 0.0,
            over_x,
            over_y,
            pending: false,
        }
    }

    pub fn finish_drag(&mut self, _tree: &mut Tree, _taken: bool) -> bool {
        false
    }

    pub fn drag_end(&mut self) -> bool {
        false
    }

    pub fn wheel(&mut self, _tree: &mut Tree, _dx: f32, _dy: f32) {}

    pub fn tick(&mut self, tree: &mut Tree) -> bool {
        self.refresh_boxes(tree);
        if let Some(id) = self.viewport
            && let Ok((x, y)) = tree.scroll_offset(id)
        {
            self.off_x = x;
            self.off_y = y;
        }
        false
    }

    pub fn consume(&mut self, _tree: &mut Tree, _overflow_x: f32, _overflow_y: f32) -> bool {
        false
    }

    pub fn query(&self, tree: &Tree, cmd: i32) -> (f32, f32) {
        match cmd {
            1 => self.limits(),
            2 => self
                .viewport
                .and_then(|id| tree.overscroll(id).ok())
                .unwrap_or((0.0, 0.0)),
            _ => self
                .viewport
                .and_then(|id| tree.scroll_offset(id).ok())
                .unwrap_or((self.off_x, self.off_y)),
        }
    }
}
