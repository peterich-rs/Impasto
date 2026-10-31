//! Virtual list state machine. Tick reads the tree directly.

use std::collections::HashMap;

use crate::tree::{NodeId, Tree};

use super::diff::{ListOp, ShownRow, diff_rows};
use super::prefix::PrefixHeights;
use super::window::{
    HEIGHT_EPS, MIN_THUMB, WindowRange, estimate_height, near_end, stabilize_window,
    sticky_translate, thumb_length, thumb_offset, window_covers, window_range_for_offset,
};

/// Scroll offset and viewport size sampled together for one placement.
#[derive(Clone, Copy)]
struct PlacedViewport {
    offset_x: f32,
    offset_y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone, Debug)]
pub struct ListItem {
    pub key: String,
    pub ty: String,
    pub sticky: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ListNodes {
    pub viewport: Option<NodeId>,
    pub content: Option<NodeId>,
    pub top: Option<NodeId>,
    pub bottom: Option<NodeId>,
    pub rows: Option<NodeId>,
    pub v_thumb: Option<NodeId>,
    pub h_thumb: Option<NodeId>,
    pub pin: Option<NodeId>,
    pub v_size: Option<NodeId>,
    pub h_size: Option<NodeId>,
}

#[derive(Clone, Debug)]
pub struct Chrome {
    pub thumb_on: bool,
    pub thumb_len: f32,
    pub thumb_off: f32,
    pub h_on: bool,
    pub h_len: f32,
    pub h_off: f32,
    pub pin_index: i32,
    pub pin_key: String,
    pub pin_y: f32,
    pub extent: f32,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            thumb_on: false,
            thumb_len: MIN_THUMB,
            thumb_off: 0.0,
            h_on: false,
            h_len: MIN_THUMB,
            h_off: 0.0,
            pin_index: -1,
            pin_key: String::new(),
            pin_y: 0.0,
            extent: 0.0,
        }
    }
}

impl Chrome {
    /// Visibility, lengths, pin identity, and extent. Offsets are written on the nodes.
    pub fn signal_changed(&self, next: &Chrome) -> bool {
        self.thumb_on != next.thumb_on
            || self.h_on != next.h_on
            || self.pin_index != next.pin_index
            || self.pin_key != next.pin_key
            || (self.thumb_len - next.thumb_len).abs() > HEIGHT_EPS
            || (self.h_len - next.h_len).abs() > HEIGHT_EPS
            || (self.extent - next.extent).abs() > HEIGHT_EPS
    }
}

struct Mounted {
    node: NodeId,
    index: usize,
}

pub struct ListDriver {
    overscan: usize,
    estimate: f32,
    scale: f32,
    blocked: bool,
    nodes: ListNodes,
    items: Vec<ListItem>,
    staging: Vec<ListItem>,
    prefix: PrefixHeights,
    cache: HashMap<String, f32>,
    mounted: HashMap<String, Mounted>,
    range: WindowRange,
    shown: Vec<ShownRow>,
    placed_w: f32,
    placed_h: f32,
    previous_offset: f32,
    end_length: i32,
    content_w: f32,
    content_view_w: f32,
    chrome: Chrome,
    signaled: Chrome,
    force: bool,
    placed: bool,
}

impl ListDriver {
    pub fn new(overscan: i32, estimate: f32) -> Self {
        Self {
            overscan: overscan.max(0) as usize,
            estimate,
            scale: 1.0,
            blocked: false,
            nodes: ListNodes::default(),
            items: Vec::new(),
            staging: Vec::new(),
            prefix: PrefixHeights::default(),
            cache: HashMap::new(),
            mounted: HashMap::new(),
            range: WindowRange::EMPTY,
            shown: Vec::new(),
            placed_w: -1.0,
            placed_h: -1.0,
            previous_offset: 0.0,
            end_length: -1,
            content_w: -1.0,
            content_view_w: -1.0,
            chrome: Chrome::default(),
            signaled: Chrome::default(),
            force: true,
            placed: false,
        }
    }

    pub fn bind(&mut self, nodes: ListNodes) {
        if nodes.viewport != self.nodes.viewport {
            self.mounted.clear();
            self.placed = false;
            self.placed_w = -1.0;
            self.placed_h = -1.0;
        }
        self.nodes.viewport = nodes.viewport.or(self.nodes.viewport);
        self.nodes.content = nodes.content.or(self.nodes.content);
        self.nodes.top = nodes.top.or(self.nodes.top);
        self.nodes.bottom = nodes.bottom.or(self.nodes.bottom);
        self.nodes.rows = nodes.rows.or(self.nodes.rows);
        self.nodes.v_thumb = nodes.v_thumb.or(self.nodes.v_thumb);
        self.nodes.h_thumb = nodes.h_thumb.or(self.nodes.h_thumb);
        self.nodes.pin = nodes.pin.or(self.nodes.pin);
        self.nodes.v_size = nodes.v_size.or(self.nodes.v_size);
        self.nodes.h_size = nodes.h_size.or(self.nodes.h_size);
        self.force = true;
    }

    pub fn configure(&mut self, estimate: f32, overscan: i32, blocked: bool) {
        self.estimate = estimate;
        self.overscan = overscan.max(0) as usize;
        self.blocked = blocked;
        self.force = true;
    }

    pub fn begin_items(&mut self) {
        self.staging.clear();
    }

    pub fn push_item(&mut self, item: ListItem) {
        if self.staging.iter().any(|row| row.key == item.key) {
            return;
        }
        self.staging.push(item);
    }

    pub fn end_items(&mut self) {
        self.items = std::mem::take(&mut self.staging);
        self.retain_cache();
        self.rebuild_prefix();
        self.force = true;
    }

    pub fn note_row(&mut self, key: &str, node: Option<NodeId>) {
        let Some(node) = node else {
            self.mounted.remove(key);
            return;
        };
        let index = self
            .items
            .iter()
            .position(|item| item.key == key)
            .unwrap_or(0);
        self.mounted.insert(key.to_owned(), Mounted { node, index });
    }

    pub fn tick(&mut self, tree: &mut Tree) -> (Vec<ListOp>, Option<Chrome>, bool, bool) {
        let Some(viewport) = self.nodes.viewport else {
            return (Vec::new(), None, false, false);
        };
        let Ok(view) = tree.rect(viewport) else {
            return (Vec::new(), None, false, false);
        };
        self.apply_scale(tree.text_scale());
        let pending_measure = self.measure(tree);
        let Ok((offset_x, offset_y)) = tree.scroll_offset(viewport) else {
            return (Vec::new(), None, false, false);
        };
        let size_changed = (view.w - self.placed_w).abs() > HEIGHT_EPS
            || (view.h - self.placed_h).abs() > HEIGHT_EPS;
        if view.h > 0.0 {
            self.placed_w = view.w;
            self.placed_h = view.h;
            self.placed = true;
        }
        let fling_y = tree.fling_target(viewport).map(|pair| pair.1);
        let refresh_width = size_changed || self.force || self.content_w < 0.0;
        let placed = PlacedViewport {
            offset_x,
            offset_y,
            w: view.w,
            h: view.h,
        };
        let must_place = self.force
            || pending_measure
            || size_changed
            || !window_covers(&self.prefix, self.range, offset_y, view.h);
        if must_place {
            self.place(tree, placed, fling_y, refresh_width);
        } else {
            self.previous_offset = offset_y;
            self.write_chrome(tree, placed, false);
        }
        let ops = self.take_ops();
        let end = self.maybe_end(self.range.end);
        let chrome = self.changed_chrome();
        let pending = !self.blocked
            && (pending_measure
                || self.needs_measure()
                || (!self.placed && !self.items.is_empty()));
        (ops, chrome, end, pending)
    }

    fn row_estimate(&self) -> f32 {
        estimate_height(self.estimate) * self.scale
    }

    fn rebuild_prefix(&mut self) {
        let estimate = self.row_estimate();
        let heights: Vec<f32> = self
            .items
            .iter()
            .map(|item| self.cache.get(&item.key).copied().unwrap_or(estimate))
            .collect();
        self.prefix.rebuild(&heights);
    }

    fn retain_cache(&mut self) {
        self.cache
            .retain(|key, _| self.items.iter().any(|item| item.key == *key));
    }

    fn apply_scale(&mut self, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 || (scale - self.scale).abs() <= f32::EPSILON {
            return;
        }
        let ratio = if self.scale > 0.0 {
            scale / self.scale
        } else {
            scale
        };
        self.scale = scale;
        for height in self.cache.values_mut() {
            *height *= ratio;
        }
        self.rebuild_prefix();
        self.force = true;
    }

    fn measure(&mut self, tree: &Tree) -> bool {
        let mut pending = false;
        let mut changed = false;
        let keys: Vec<String> = self.mounted.keys().cloned().collect();
        let mut dead = Vec::new();
        for key in keys {
            let Some(mounted) = self.mounted.get(&key) else {
                continue;
            };
            let Ok(box_rect) = tree.rect(mounted.node) else {
                dead.push(key);
                continue;
            };
            if box_rect.h == 0.0 {
                pending = true;
                continue;
            }
            let index = self
                .items
                .iter()
                .position(|item| item.key == key)
                .unwrap_or(mounted.index);
            if let Some(slot) = self.mounted.get_mut(&key) {
                slot.index = index;
            }
            let prev = self.prefix.height_at(index);
            self.cache.insert(key, box_rect.h);
            if (box_rect.h - prev).abs() > HEIGHT_EPS && index < self.prefix.len() {
                self.prefix.add(index, box_rect.h - prev);
                changed = true;
            }
        }
        for key in dead {
            self.mounted.remove(&key);
        }
        if changed {
            self.force = true;
        }
        pending
    }

    fn place(
        &mut self,
        tree: &mut Tree,
        view: PlacedViewport,
        fling_y: Option<f32>,
        refresh_width: bool,
    ) {
        let PlacedViewport {
            offset_y,
            h: view_h,
            ..
        } = view;
        let delta = offset_y - self.previous_offset;
        self.previous_offset = offset_y;
        let lead = if delta == 0.0 || view_h <= 0.0 {
            None
        } else {
            Some(offset_y + delta.signum() * view_h)
        };
        let far = match (fling_y, lead) {
            (Some(flung), None) => Some(flung),
            (Some(flung), Some(lead)) if delta > 0.0 => Some(flung.max(lead)),
            (Some(flung), Some(lead)) => Some(flung.min(lead)),
            (None, _) => lead,
        };
        let next = window_range_for_offset(&self.prefix, offset_y, view_h, self.overscan, far);
        self.range = stabilize_window(&self.prefix, self.range, next, self.overscan);
        self.write_chrome(tree, view, refresh_width);
    }

    fn write_chrome(&mut self, tree: &mut Tree, view: PlacedViewport, refresh_width: bool) {
        let PlacedViewport {
            offset_x,
            offset_y,
            w: view_w,
            h: view_h,
        } = view;
        let content = self.prefix.total();
        let overflow = content > view_h + HEIGHT_EPS && view_h > 0.0 && content > 0.0;
        if overflow {
            self.chrome.thumb_on = true;
            self.chrome.thumb_len = thumb_length(view_h, content);
            self.chrome.thumb_off = thumb_offset(view_h, content, offset_y);
        } else {
            self.chrome.thumb_on = false;
            self.chrome.thumb_len = MIN_THUMB;
            self.chrome.thumb_off = 0.0;
        }
        if refresh_width || (view_w - self.content_view_w).abs() > HEIGHT_EPS {
            self.content_view_w = view_w;
            if let Some(content_id) = self.nodes.content
                && let Ok(body) = tree.rect(content_id)
            {
                self.content_w = body.w;
            }
        }
        if self.content_w > view_w + HEIGHT_EPS && view_w > 0.0 {
            self.chrome.h_on = true;
            self.chrome.h_len = thumb_length(view_w, self.content_w);
            self.chrome.h_off = thumb_offset(view_w, self.content_w, offset_x);
        } else {
            self.chrome.h_on = false;
            self.chrome.h_len = MIN_THUMB;
            self.chrome.h_off = 0.0;
        }
        self.write_sticky(offset_y);
        self.chrome.extent = content;
        self.write_nodes(tree);
    }

    fn write_sticky(&mut self, offset_y: f32) {
        let mut current = -1i32;
        for (index, item) in self.items.iter().enumerate() {
            if !item.sticky {
                continue;
            }
            if self.prefix.item_start(index) <= offset_y {
                current = crate::cast::i32_from_usize(index);
            } else {
                break;
            }
        }
        if current < 0 {
            self.chrome.pin_index = -1;
            self.chrome.pin_key.clear();
            self.chrome.pin_y = 0.0;
            return;
        }
        let mut next_start = None;
        for index in (current as usize + 1)..self.items.len() {
            if self.items[index].sticky {
                next_start = Some(self.prefix.item_start(index));
                break;
            }
        }
        let height = self.prefix.height_at(current as usize);
        self.chrome.pin_y = sticky_translate(next_start, offset_y, height);
        self.chrome.pin_index = current;
        let key = self.items[current as usize].key.as_str();
        if self.chrome.pin_key != key {
            self.chrome.pin_key = key.to_owned();
        }
    }

    fn write_nodes(&self, tree: &mut Tree) {
        if let Some(id) = self.nodes.top {
            let _ = tree.set_size(id, 1.0, self.range.top.max(0.0));
        }
        if let Some(id) = self.nodes.bottom {
            let _ = tree.set_size(id, 1.0, self.range.bottom.max(0.0));
        }
        if let Some(id) = self.nodes.content {
            let _ = tree.set_min_size(id, 0.0, self.prefix.total().max(0.0));
        }
        if self.chrome.thumb_on {
            if let Some(id) = self.nodes.v_thumb {
                let _ = tree.set_transform(id, 0.0, self.chrome.thumb_off, 0.0, 1.0);
            }
            if let Some(id) = self.nodes.v_size {
                let _ = tree.set_size(id, 3.0, self.chrome.thumb_len.max(1.0));
            }
        }
        if self.chrome.h_on {
            if let Some(id) = self.nodes.h_thumb {
                let _ = tree.set_transform(id, self.chrome.h_off, 0.0, 0.0, 1.0);
            }
            if let Some(id) = self.nodes.h_size {
                let _ = tree.set_size(id, self.chrome.h_len.max(1.0), 3.0);
            }
        }
        if let Some(id) = self.nodes.pin {
            let _ = tree.set_transform(id, 0.0, self.chrome.pin_y, 0.0, 1.0);
        }
    }

    fn take_ops(&mut self) -> Vec<ListOp> {
        let mut next = Vec::new();
        let end = self.range.end.min(self.items.len());
        let start = self.range.start.min(end);
        for index in start..end {
            let item = &self.items[index];
            next.push(ShownRow {
                key: item.key.clone(),
                ty: item.ty.clone(),
                index: crate::cast::u32_from_usize(index),
            });
        }
        let force = self.force;
        let ops = diff_rows(&self.shown, &next, force);
        if !ops.is_empty() {
            self.shown = next;
        }
        self.force = false;
        ops
    }

    fn maybe_end(&mut self, end: usize) -> bool {
        if self.blocked || self.items.is_empty() {
            return false;
        }
        let length = self.items.len();
        if !near_end(end, length, self.overscan) {
            return false;
        }
        if self.end_length == crate::cast::i32_from_usize(length) {
            return false;
        }
        self.end_length = crate::cast::i32_from_usize(length);
        true
    }

    fn needs_measure(&self) -> bool {
        let end = self.range.end.min(self.items.len());
        let start = self.range.start.min(end);
        (start..end).any(|index| {
            self.items
                .get(index)
                .is_some_and(|item| !self.cache.contains_key(&item.key))
        })
    }

    fn changed_chrome(&mut self) -> Option<Chrome> {
        // `chrome` is the live sample. Compare against the last signal we published.
        // The published copy lives in `signaled`.
        if self.signaled.signal_changed(&self.chrome) {
            self.signaled = self.chrome.clone();
            Some(self.chrome.clone())
        } else {
            None
        }
    }
}
