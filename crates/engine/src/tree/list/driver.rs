//! Virtual list state machine. Tick reads the tree directly.

use std::collections::HashMap;

use crate::tree::compositor_motion::{ListCompositorLink, ListCompositorWindow};
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
}

pub struct ListDriver {
    overscan: usize,
    estimate: f32,
    scale: f32,
    blocked: bool,
    nodes: ListNodes,
    items: Vec<ListItem>,
    staging: Vec<ListItem>,
    staging_indices: HashMap<String, usize>,
    indices: HashMap<String, usize>,
    pending_anchor: Option<(f32, f32)>,
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
    /// Consecutive ticks that corrected a measured row. Reset when a tick does not.
    measure_burst: u32,
    /// Leading rows whose laid-out height is still zero.
    zero_rows: u32,
    /// The blank-top warning is edge-triggered.
    gap_open: bool,
}

impl ListDriver {
    pub(super) fn viewport(&self) -> Option<NodeId> {
        self.nodes.viewport
    }

    pub(super) fn compositor_window(&self, tree: &Tree) -> Option<ListCompositorWindow> {
        let viewport = self.nodes.viewport?;
        let view = tree.rect(viewport).ok()?;
        let (_, offset) = tree.scroll_offset(viewport).ok()?;
        if !self.placed || self.blocked || !tree.motion_active() || view.h <= 0.0 {
            return None;
        }
        let rows = self.items.get(self.range.start..self.range.end)?;
        let (mut top, mut bottom) = (None, 0.0_f32);
        for item in rows {
            let rect = tree.rect(self.mounted.get(&item.key)?.node).ok()?;
            let y = rect.y - view.y;
            if rect.h <= 0.0 || top.is_some() && y > bottom + HEIGHT_EPS {
                return None;
            }
            top.get_or_insert(y);
            bottom = y + rect.h;
        }
        let top = top?;
        let slack = (view.h * 0.25).min(((bottom - top - view.h).max(0.0)) * 0.5);
        let mut low = top;
        let mut high = bottom - view.h;
        let prefetch_edges = [
            (self.range.start > 0).then_some(low + slack),
            (self.range.end < self.items.len()).then_some(high - slack),
        ];
        let pin = usize::try_from(self.chrome.pin_index).ok();
        let next_start = self
            .items
            .iter()
            .enumerate()
            .skip(pin.map_or(0, |i| i + 1))
            .find(|(_, item)| item.sticky)
            .map(|(index, _)| self.prefix.item_start(index));
        if let Some(start) = next_start {
            high = high.min(start);
        }
        if let Some(index) = pin {
            low = low.max(self.prefix.item_start(index));
        }
        let limit = tree.scroll_limits(viewport).1;
        let covered_start = offset >= low || (offset == 0.0 && low.abs() <= HEIGHT_EPS);
        let covered_end = offset <= high || (offset == limit && (limit - high).abs() <= HEIGHT_EPS);
        if !(covered_start && covered_end) {
            return None;
        }
        let sticky = match (pin, next_start) {
            (Some(index), Some(start)) => Some(ListCompositorLink {
                node: self.nodes.pin?,
                factor: -1.0,
                offset: start - self.prefix.height_at(index),
                maximum: Some(0.0),
            }),
            _ => None,
        };
        let thumb = |node, view: f32, content, visible| {
            if visible {
                Some(ListCompositorLink {
                    node: node?,
                    factor: thumb_offset(view, content, 1.0),
                    offset: 0.0,
                    maximum: None,
                })
            } else {
                None
            }
        };
        let vertical_thumb = thumb(
            self.nodes.v_thumb,
            view.h,
            self.prefix.total(),
            self.chrome.thumb_on,
        );
        let horizontal_thumb = thumb(self.nodes.h_thumb, view.w, self.content_w, self.chrome.h_on);
        if (self.chrome.thumb_on && vertical_thumb.is_none())
            || (self.chrome.h_on && horizontal_thumb.is_none())
        {
            return None;
        }
        Some(ListCompositorWindow {
            bounds: [low, high],
            prefetch_edges,
            horizontal_thumb,
            vertical_thumb,
            sticky,
        })
    }

    pub fn new(overscan: i32, estimate: f32) -> Self {
        Self {
            overscan: overscan.max(0) as usize,
            estimate,
            scale: 1.0,
            blocked: false,
            nodes: ListNodes::default(),
            items: Vec::new(),
            staging: Vec::new(),
            staging_indices: HashMap::new(),
            indices: HashMap::new(),
            pending_anchor: None,
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
            measure_burst: 0,
            zero_rows: 0,
            gap_open: false,
        }
    }

    pub fn bind(&mut self, nodes: ListNodes) {
        if nodes.viewport != self.nodes.viewport {
            self.mounted.clear();
            self.placed = false;
            self.pending_anchor = None;
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
        let overscan = overscan.max(0) as usize;
        if self.estimate == estimate && self.overscan == overscan && self.blocked == blocked {
            return;
        }
        self.estimate = estimate;
        self.overscan = overscan;
        self.blocked = blocked;
        self.force = true;
    }

    pub fn begin_items(&mut self) {
        self.staging.clear();
        self.staging_indices.clear();
    }

    pub fn push_item(&mut self, item: ListItem) {
        // The first occurrence owns the row identity.
        if !self.staging_indices.contains_key(&item.key) {
            self.staging_indices
                .insert(item.key.clone(), self.staging.len());
            self.staging.push(item);
        }
    }

    pub fn end_items(&mut self) {
        if self.staging == self.items {
            return;
        }
        let from = self
            .pending_anchor
            .map_or(self.previous_offset, |(from, _)| from);
        let offset = self
            .pending_anchor
            .map_or(self.previous_offset, |(_, to)| to);
        let anchor = self
            .placed
            .then(|| self.anchor_after_update(offset))
            .flatten();
        std::mem::swap(&mut self.items, &mut self.staging);
        std::mem::swap(&mut self.indices, &mut self.staging_indices);
        self.begin_items();
        self.cache.retain(|key, _| self.indices.contains_key(key));
        self.rebuild_prefix();
        self.pending_anchor =
            anchor.map(|(index, inside)| (from, self.anchored_offset(index, inside)));
        self.force = true;
    }

    fn anchor_after_update(&self, offset: f32) -> Option<(usize, f32)> {
        let (old, inside) = self.anchor(offset)?;
        if let Some(&index) = self.staging_indices.get(&self.items[old].key) {
            return Some((index, inside));
        }
        // Walk outward in the former order, preferring the following row on a tie.
        for distance in 1..self.items.len() {
            for neighbor in [old.checked_add(distance), old.checked_sub(distance)]
                .into_iter()
                .flatten()
            {
                if let Some(index) = self
                    .items
                    .get(neighbor)
                    .and_then(|item| self.staging_indices.get(&item.key))
                {
                    return Some((*index, inside));
                }
            }
        }
        None
    }

    fn anchor(&self, offset: f32) -> Option<(usize, f32)> {
        let index = self
            .prefix
            .find_index(offset)
            .min(self.items.len().checked_sub(1)?);
        Some((index, offset - self.prefix.item_start(index)))
    }

    fn anchored_offset(&self, index: usize, inside: f32) -> f32 {
        self.prefix.item_start(index)
            + inside.clamp(0.0, (self.prefix.height_at(index) - HEIGHT_EPS).max(0.0))
    }

    fn own_rows(&self, tree: &mut Tree) {
        for mounted in self.mounted.values() {
            let _ = tree.set_picture_owner(mounted.node, true);
        }
    }

    pub fn note_row(&mut self, key: &str, node: Option<NodeId>) {
        let Some(node) = node else {
            self.mounted.remove(key);
            return;
        };
        if self.indices.contains_key(key) {
            self.mounted.insert(key.to_owned(), Mounted { node });
        }
    }

    pub fn tick(&mut self, tree: &mut Tree) -> (Vec<ListOp>, Option<Chrome>, bool, bool) {
        let Some(viewport) = self.nodes.viewport else {
            return (Vec::new(), None, false, false);
        };
        self.own_rows(tree);
        let Ok(view) = tree.rect(viewport) else {
            return (Vec::new(), None, false, false);
        };
        let Ok((offset_x, mut offset_y)) = tree.scroll_offset(viewport) else {
            return (Vec::new(), None, false, false);
        };
        if view.h <= 0.0 {
            return (Vec::new(), None, false, false);
        }
        let anchor_enabled = self.placed
            && tree.scroll_phase(viewport).ok() == Some(crate::motion::ScrollPhase::Idle);
        let updated = self
            .pending_anchor
            .take()
            .filter(|(from, _)| anchor_enabled && (*from - offset_y).abs() <= HEIGHT_EPS);
        let anchor = anchor_enabled
            .then(|| self.anchor(updated.map_or(offset_y, |(_, to)| to)))
            .flatten();
        self.apply_scale(tree.text_scale());
        let pending_measure = self.measure(tree);
        if let Some((index, inside)) = anchor {
            let target = self
                .anchored_offset(index, inside)
                .clamp(0.0, (self.prefix.total() - view.h).max(0.0));
            if (target - offset_y).abs() > HEIGHT_EPS {
                let _ = tree.set_content_min(viewport, self.prefix.total());
                if tree.set_scroll_offset(viewport, offset_x, target).is_ok() {
                    offset_y = target;
                    self.previous_offset = target;
                }
            }
        }
        let size_changed = (view.w - self.placed_w).abs() > HEIGHT_EPS
            || (view.h - self.placed_h).abs() > HEIGHT_EPS;
        if view.h > 0.0 {
            self.placed_w = view.w;
            self.placed_h = view.h;
            self.placed = true;
        }
        let predicted = tree.list_scroll_prediction(viewport);
        let fling_y = tree.fling_target(viewport).map(|pair| pair.1).or(predicted);
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
            || !window_covers(&self.prefix, self.range, offset_y, view.h)
            || predicted.is_some_and(|future| {
                (future < offset_y && self.range.start > 0
                    || future > offset_y && self.range.end < self.items.len())
                    && !window_covers(&self.prefix, self.range, future, view.h)
            });
        if std::env::var("IMPASTO_TEST_LOG").is_ok() {
            let mut chain = String::new();
            let mut cursor = Some(viewport);
            for _ in 0..8 {
                let Some(id) = cursor else { break };
                let rect = tree.rect(id).ok();
                chain.push_str(&format!(
                    " #{:x} {:?}",
                    id.to_raw(),
                    rect.map(|r| (r.w, r.h))
                ));
                cursor = tree.parent(id).ok().flatten();
            }
            if let Some(top) = tree.parent(viewport).ok().flatten() {
                let mut child = tree.first_child(top).ok().flatten();
                chain.push_str(" | children");
                while let Some(id) = child {
                    let rect = tree.rect(id).ok();
                    chain.push_str(&format!(
                        " #{:x}{:?}",
                        id.to_raw(),
                        rect.map(|r| (r.w, r.h))
                    ));
                    child = tree.next_sibling(id).ok().flatten();
                }
            }
            eprintln!("[probe] list chain{chain} items={}", self.prefix.len());
        }
        if must_place {
            self.place(tree, placed, fling_y, refresh_width);
        } else {
            let delta = offset_y - self.previous_offset;
            self.previous_offset = offset_y;
            self.write_chrome(tree, placed, false);
            if view.h > 0.0 && delta.abs() > view.h * 0.5 {
                tracing::info!(
                    target: "impasto::list",
                    offset = offset_y,
                    delta,
                    start = self.range.start,
                    end = self.range.end,
                    top = self.range.top,
                    "list hold"
                );
            }
        }
        self.note_gap(offset_y, view.h);
        let ops = self.take_ops();
        if !ops.is_empty() {
            let (detach, adopt, set, order) = count_ops(&ops);
            tracing::info!(
                target: "impasto::list",
                start = self.range.start,
                end = self.range.end,
                detach,
                adopt,
                set,
                order,
                mounted = self.mounted.len(),
                "list ops"
            );
        }
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
        let mut changed_rows = 0u32;
        let mut shift = 0.0f32;
        let mut zeros = 0u32;
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
                zeros += 1;
                continue;
            }
            let Some(&index) = self.indices.get(&key) else {
                continue;
            };
            let prev = self.prefix.height_at(index);
            self.cache.insert(key, box_rect.h);
            let delta = (box_rect.h - prev).abs();
            if delta > HEIGHT_EPS && index < self.prefix.len() {
                self.prefix.add(index, box_rect.h - prev);
                changed = true;
                changed_rows += 1;
                shift += delta;
            }
        }
        for key in dead {
            self.mounted.remove(&key);
        }
        if zeros != self.zero_rows {
            tracing::warn!(
                target: "impasto::list",
                rows = zeros,
                mounted = self.mounted.len(),
                "list zero-height rows"
            );
            self.zero_rows = zeros;
        }
        if changed {
            self.force = true;
            self.measure_burst = self.measure_burst.saturating_add(1);
            if self.measure_burst == 1 || self.measure_burst.is_multiple_of(10) {
                tracing::info!(
                    target: "impasto::list",
                    rows = changed_rows,
                    shift,
                    burst = self.measure_burst,
                    "list measure"
                );
            }
        } else {
            self.measure_burst = 0;
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
        let previous = self.range;
        self.range = stabilize_window(&self.prefix, self.range, next, self.overscan);
        let top_changed = (self.range.top - previous.top).abs() > HEIGHT_EPS
            || (self.range.bottom - previous.bottom).abs() > HEIGHT_EPS;
        if self.range.start != previous.start || self.range.end != previous.end || top_changed {
            tracing::info!(
                target: "impasto::list",
                offset = offset_y,
                view = view_h,
                start = self.range.start,
                end = self.range.end,
                next_start = next.start,
                next_end = next.end,
                top = self.range.top,
                bottom = self.range.bottom,
                items = self.items.len(),
                mounted = self.mounted.len(),
                prefix = self.prefix.total(),
                lead = far.unwrap_or(-1.0),
                delta,
                "list window"
            );
        }
        self.write_chrome(tree, view, refresh_width);
    }

    /// Viewport starts in the top spacer, or the leading rows were collapsed out of the window.
    fn note_gap(&mut self, offset_y: f32, view_h: f32) {
        let collapsed = self.range.start > 0
            && offset_y <= 1.0
            && self.prefix.item_start(self.range.start) <= offset_y + 1.0;
        let blank = self.range.start > 0 && offset_y + 1.0 < self.range.top;
        let gap = collapsed || blank;
        if gap && !self.gap_open {
            let head = self.range.start.min(self.prefix.len()).min(4);
            let heights = (0..head)
                .map(|index| format!("{:.1}", self.prefix.height_at(index)))
                .collect::<Vec<_>>()
                .join(",");
            tracing::warn!(
                target: "impasto::list",
                offset = offset_y,
                view = view_h,
                start = self.range.start,
                end = self.range.end,
                top = self.range.top,
                prefix = self.prefix.total(),
                heights = heights.as_str(),
                collapsed,
                blank,
                "list top gap"
            );
        }
        self.gap_open = gap;
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
        self.cull_pinned_row(tree);
        self.chrome.extent = content;
        self.write_nodes(tree);
    }

    /// The pinned header is also mounted in the window. Hide that copy so it
    /// does not paint twice. The measured height stays in the prefix.
    fn cull_pinned_row(&self, tree: &mut Tree) {
        let pin = self.chrome.pin_key.as_str();
        let ready = self.nodes.pin.is_some_and(|id| {
            tree.rect(id).is_ok_and(|rect| rect.w > 0.0 && rect.h > 0.0)
                && tree.first_child(id).is_ok_and(|child| child.is_some())
        });
        for (key, mounted) in &self.mounted {
            let cull = ready && !pin.is_empty() && key == pin;
            let _ = tree.set_cull_paint(mounted.node, cull);
        }
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
            if self.content_view_w > 0.0
                && let Ok(index) = usize::try_from(self.chrome.pin_index)
            {
                let _ = tree.set_size(id, self.content_view_w, self.prefix.height_at(index));
            }
            let _ = tree.set_transform(id, 0.0, self.chrome.pin_y, 0.0, 1.0);
        }
    }

    fn take_ops(&mut self) -> Vec<ListOp> {
        let mut next = Vec::new();
        let end = self.range.end.min(self.items.len());
        let start = self.range.start.min(end);
        for (index, item) in self.items.iter().enumerate().take(end).skip(start) {
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

fn count_ops(ops: &[ListOp]) -> (usize, usize, usize, usize) {
    let mut detach = 0;
    let mut adopt = 0;
    let mut set = 0;
    let mut order = 0;
    for op in ops {
        match op {
            ListOp::Detach { .. } => detach += 1,
            ListOp::Adopt { .. } => adopt += 1,
            ListOp::Set { .. } => set += 1,
            ListOp::Place { .. } => order += 1,
        }
    }
    (detach, adopt, set, order)
}
