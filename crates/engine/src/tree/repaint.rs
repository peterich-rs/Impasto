//! Opt-in accounting for scene recording and remote diagnostic overlays.

use std::collections::HashMap;

use super::{NodeId, Rect, Tree};
use crate::input::{Phase, PointerEvent};
use crate::scene::{Command, PictureId};

/// The recording work actually kept by the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaintPath {
    FastLayerPatch,
    PicturePatch,
    FullPictureRebuild,
    CanvasRedraw,
}

impl PaintPath {
    pub fn name(self) -> &'static str {
        match self {
            Self::FastLayerPatch => "fast_layer_patch",
            Self::PicturePatch => "picture_patch",
            Self::FullPictureRebuild => "full_picture_rebuild",
            Self::CanvasRedraw => "canvas_redraw",
        }
    }
}

bitflags::bitflags! {
    /// Coalesced changes that preceded a recorded view.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct RepaintReason: u16 {
        const STYLE_PAINT = 1 << 0;
        const LAYOUT_GEOMETRY = 1 << 1;
        const TEXT_CONTENT = 1 << 2;
        const TRANSFORM_OR_OPACITY = 1 << 3;
        const CLIP_OR_RADIUS = 1 << 4;
        const EFFECT_LAYER = 1 << 5;
        const CANVAS_CALLBACK = 1 << 6;
        const ANCESTOR_INVALIDATED = 1 << 7;
        const TREE_STRUCTURE = 1 << 8;
        const INITIAL_MOUNT = 1 << 9;
    }
}

/// Cumulative work for one generation of a view.
#[derive(Clone, Debug)]
pub struct NodeRepaintStats {
    pub node: NodeId,
    pub total_repaints: u64,
    pub total_layouts: u64,
    pub path_counts: [u64; 4],
    pub last_frame: u64,
    pub last_path: Option<PaintPath>,
    pub last_reason: RepaintReason,
    pub recent_mask: u64,
}

/// One view's coalesced recording work in a frame.
#[derive(Clone, Debug)]
pub struct FrameRepaintRecord {
    pub node: NodeId,
    pub kind: &'static str,
    pub rect: Rect,
    pub path: PaintPath,
    pub reason: RepaintReason,
    pub picture_id: Option<PictureId>,
    pub command_count: usize,
    pub source: Option<(String, String, u32, u32)>,
}

pub const FRAME_REPAINT_CAP: usize = 256;

/// Bounded details and untruncated counts from one UI record.
#[derive(Clone, Debug)]
pub struct FrameRepaintBatch {
    pub frame_number: u64,
    pub total_repainted_views: usize,
    pub overflow_count: usize,
    pub path_counts: [usize; 4],
    pub entries: Vec<FrameRepaintRecord>,
}

/// Independent controls for the diagnostic scene suffix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteOverlayConfig {
    pub highlight: Option<NodeId>,
    pub pick_mode: bool,
    pub repaint_rainbow: bool,
    pub layout_scan: bool,
}

/// Latest scene hit while a remote client owns pointer picking.
#[derive(Clone, Debug)]
pub struct PickedViewEvent {
    pub chain: Vec<NodeId>,
    pub x: f32,
    pub y: f32,
    pub final_pick: bool,
}

#[derive(Default)]
pub(super) struct RemoteOverlayState {
    config: RemoteOverlayConfig,
    rainbow: Vec<RainbowBox>,
    picked: Option<PickedViewEvent>,
}

struct RainbowBox {
    rect: Rect,
    path: PaintPath,
    frames: u8,
}

const RAINBOW_FRAMES: u8 = 24;
pub(super) const OVERLAY_PICTURE_ID: PictureId = PictureId(u64::MAX - 1);

#[derive(Default)]
pub(super) struct RepaintTracker {
    frame: u64,
    nodes: HashMap<NodeId, NodeRepaintStats>,
    staged: HashMap<NodeId, RepaintReason>,
    events: HashMap<NodeId, (PaintPath, Option<PictureId>, usize)>,
    /// Nodes contributing commands to the current sequential picture run.
    pub(super) run_members: Vec<NodeId>,
    batch: Option<FrameRepaintBatch>,
}

impl RepaintTracker {
    pub(super) fn retire(&mut self, index: u32) {
        self.nodes.retain(|node, _| node.index != index);
        self.staged.retain(|node, _| node.index != index);
        self.events.retain(|node, _| node.index != index);
    }

    fn stats(&mut self, node: NodeId) -> &mut NodeRepaintStats {
        self.nodes.entry(node).or_insert(NodeRepaintStats {
            node,
            total_repaints: 0,
            total_layouts: 0,
            path_counts: [0; 4],
            last_frame: 0,
            last_path: None,
            last_reason: RepaintReason::empty(),
            recent_mask: 0,
        })
    }
}

impl Tree {
    /// Activates accounting at a UI frame boundary. Detached generations are discarded.
    pub fn begin_repaint_frame(&mut self, enabled: bool, frame_number: u64) {
        if enabled || self.debug.overlay.config.repaint_rainbow {
            let tracker = self.debug.repaint.get_or_insert_with(Box::default);
            tracker.frame = frame_number;
        } else {
            self.debug.repaint = None;
        }
    }

    pub(super) fn stage_repaint(&mut self, node: NodeId, reason: RepaintReason) {
        if self.debug.repaint.is_none() || self.is_debug_owned(node) {
            return;
        }
        if let Some(tracker) = self.debug.repaint.as_mut() {
            let staged = tracker.staged.entry(node).or_default();
            let first_layout = reason.contains(RepaintReason::LAYOUT_GEOMETRY)
                && !staged.contains(RepaintReason::LAYOUT_GEOMETRY);
            *staged |= reason;
            if first_layout {
                tracker.stats(node).total_layouts += 1;
            }
        }
    }

    pub(super) fn commit_repaint(
        &mut self,
        node: NodeId,
        path: PaintPath,
        picture: Option<PictureId>,
        commands: usize,
    ) {
        if self.debug.repaint.is_none() || self.is_debug_owned(node) {
            return;
        }
        if let Some(tracker) = self.debug.repaint.as_mut() {
            let entry = tracker
                .events
                .entry(node)
                .or_insert((path, picture, commands));
            if path > entry.0 || (path == entry.0 && picture.is_some()) {
                *entry = (path, picture, commands);
            }
        }
    }

    pub(super) fn commit_picture_repaint(
        &mut self,
        owner: NodeId,
        path: PaintPath,
        picture: PictureId,
        commands: usize,
    ) {
        let Some(tracker) = self.debug.repaint.as_ref() else {
            return;
        };
        let affected: Vec<_> = tracker
            .staged
            .keys()
            .copied()
            .filter(|node| *node != owner && self.recording_owner(*node) == Some(owner))
            .collect();
        self.commit_repaint(owner, path, Some(picture), commands);
        for node in affected {
            self.commit_repaint(node, path, Some(picture), commands);
        }
    }

    fn recording_owner(&self, mut node: NodeId) -> Option<NodeId> {
        loop {
            let parent = self.get(node)?.parent;
            if parent.is_none() || self.renders_layer_node(node) || self.is_picture_owner(node) {
                return Some(node);
            }
            node = parent?;
        }
    }

    pub(super) fn finish_repaints(&mut self) {
        let Some(mut tracker) = self.debug.repaint.take() else {
            return;
        };
        let total_repainted_views = tracker.events.len();
        let mut path_counts = [0; 4];
        let frame = tracker.frame;
        let mut events = std::mem::take(&mut tracker.events);
        let mut ranked = Vec::with_capacity(events.len());
        for (node, (path, picture_id, command_count)) in events.drain() {
            let Some(view) = self.get(node) else { continue };
            let reason = tracker.staged.remove(&node);
            let stats = tracker.stats(node);
            let reason = reason.unwrap_or({
                if stats.total_repaints == 0 {
                    RepaintReason::INITIAL_MOUNT
                } else {
                    RepaintReason::ANCESTOR_INVALIDATED
                }
            });
            let path = if reason.contains(RepaintReason::CANVAS_CALLBACK)
                && path != PaintPath::FastLayerPatch
            {
                PaintPath::CanvasRedraw
            } else {
                path
            };
            let elapsed = frame.saturating_sub(stats.last_frame);
            stats.recent_mask = if elapsed >= 64 {
                1
            } else {
                (stats.recent_mask << elapsed) | 1
            };
            stats.total_repaints += 1;
            stats.last_frame = frame;
            stats.last_path = Some(path);
            stats.last_reason = reason;
            stats.path_counts[path as usize] += 1;
            path_counts[path as usize] += 1;
            if let Ok(rect) = self.scene_rect(node) {
                ranked.push((
                    node,
                    view.kind_name(),
                    rect,
                    path,
                    reason,
                    picture_id,
                    command_count,
                ));
            }
        }
        tracker.events = events;
        tracker.staged.clear();
        ranked.sort_unstable_by(|left, right| {
            right
                .3
                .cmp(&left.3)
                .then_with(|| (right.2.w * right.2.h).total_cmp(&(left.2.w * left.2.h)))
                .then_with(|| left.0.to_raw().cmp(&right.0.to_raw()))
        });
        tracker.batch = (total_repainted_views > 0).then(|| FrameRepaintBatch {
            frame_number: frame,
            total_repainted_views,
            overflow_count: total_repainted_views.saturating_sub(FRAME_REPAINT_CAP),
            path_counts,
            entries: ranked
                .into_iter()
                .take(FRAME_REPAINT_CAP)
                .map(
                    |(node, kind, rect, path, reason, picture_id, command_count)| {
                        FrameRepaintRecord {
                            node,
                            kind,
                            rect,
                            path,
                            reason,
                            picture_id,
                            command_count,
                            source: self.debug.sources.get(&node.index).map(|source| {
                                (
                                    source.component.clone(),
                                    source.path.clone(),
                                    source.line,
                                    source.column,
                                )
                            }),
                        }
                    },
                )
                .collect(),
        });
        self.debug.repaint = Some(tracker);
    }

    /// Takes the completed recording batch for delivery to the host.
    pub fn take_repaints(&mut self) -> Option<FrameRepaintBatch> {
        self.debug.repaint.as_mut()?.batch.take()
    }

    /// Ranks live views by recent frequency, then lifetime frequency.
    pub fn repaint_leaderboard(&self, limit: usize) -> Vec<NodeRepaintStats> {
        let Some(tracker) = &self.debug.repaint else {
            return Vec::new();
        };
        let mut items: Vec<_> = tracker.nodes.values().cloned().collect();
        for item in &mut items {
            let elapsed = tracker.frame.saturating_sub(item.last_frame);
            item.recent_mask = if elapsed >= 64 {
                0
            } else {
                item.recent_mask << elapsed
            };
        }
        items.sort_unstable_by_key(|item| {
            (
                std::cmp::Reverse(item.recent_mask.count_ones()),
                std::cmp::Reverse(item.total_repaints),
                item.node.to_raw(),
            )
        });
        items.truncate(limit);
        items
    }

    pub(super) fn repaint_stats(&self, node: NodeId) -> Option<&NodeRepaintStats> {
        self.debug.repaint.as_ref()?.nodes.get(&node)
    }

    /// Replaces the diagnostic controls and requests a fresh scene if needed.
    pub fn set_remote_overlay(&mut self, config: RemoteOverlayConfig) {
        let previous = self.debug.overlay.config;
        if previous == config {
            return;
        }
        self.debug.overlay.config = config;
        if !config.repaint_rainbow {
            self.debug.overlay.rainbow.clear();
        }
        if previous.layout_scan != config.layout_scan {
            self.scan_overflow();
            if !self.debug_scan_enabled() {
                self.debug.issues.clear();
                self.debug.new_issues.clear();
                self.debug.seen_issues.clear();
            }
        }
        if previous.highlight != config.highlight
            || previous.repaint_rainbow != config.repaint_rainbow
        {
            self.paint_layer_value();
        }
    }

    pub(super) fn remote_scan_enabled(&self) -> bool {
        self.debug.overlay.config.layout_scan
    }

    /// True while diagnostic marks need a new opacity or removal frame.
    #[cfg(feature = "js")]
    pub(crate) fn overlay_animating(&self) -> bool {
        !self.debug.overlay.rainbow.is_empty()
    }

    pub(crate) fn overlay_visible(&self) -> bool {
        #[cfg(debug_assertions)]
        if self.debug.dev_error.is_some() {
            return true;
        }
        self.debug.overlay.config.highlight.is_some() || self.debug.overlay.config.repaint_rainbow
    }

    #[cfg(feature = "js")]
    pub(crate) fn advance_overlay(&mut self) {
        if self.overlay_animating() {
            self.paint_layer_value();
        }
    }

    pub(super) fn pick_remote_pointer(&mut self, event: PointerEvent) -> bool {
        if !self.debug.overlay.config.pick_mode {
            return false;
        }
        if matches!(event.phase, Phase::Down | Phase::Move | Phase::Up) {
            let chain = self.debug_probe_chain(event.x, event.y);
            let next = RemoteOverlayConfig {
                highlight: chain.last().copied(),
                ..self.debug.overlay.config
            };
            self.set_remote_overlay(next);
            self.debug.overlay.picked = Some(PickedViewEvent {
                chain,
                x: event.x,
                y: event.y,
                final_pick: event.phase == Phase::Up,
            });
        }
        true
    }

    pub fn take_picked_view(&mut self) -> Option<PickedViewEvent> {
        self.debug.overlay.picked.take()
    }

    pub(super) fn overlay_commands(&mut self) -> Vec<Command> {
        let highlight = self
            .debug
            .overlay
            .config
            .highlight
            .filter(|node| !self.is_debug_owned(*node))
            .and_then(|node| self.scene_rect(node).ok());
        let overlay = &mut self.debug.overlay;
        if overlay.config.repaint_rainbow
            && let Some(batch) = self
                .debug
                .repaint
                .as_ref()
                .and_then(|tracker| tracker.batch.as_ref())
        {
            overlay.rainbow.extend(
                batch
                    .entries
                    .iter()
                    .filter(|entry| entry.rect.w > 0.0 && entry.rect.h > 0.0)
                    .take(32)
                    .map(|entry| RainbowBox {
                        rect: entry.rect,
                        path: entry.path,
                        frames: RAINBOW_FRAMES,
                    }),
            );
            let excess = overlay.rainbow.len().saturating_sub(64);
            overlay.rainbow.drain(..excess);
        }
        // Retain zero-frame markers until the next record clears their last outline.
        overlay.rainbow.retain(|marker| marker.frames > 0);
        let mut commands = Vec::new();
        for marker in &mut overlay.rainbow {
            if marker.frames > 0 {
                let mut stroke = match marker.path {
                    PaintPath::FastLayerPatch => [76, 201, 140, 255],
                    PaintPath::PicturePatch => [255, 195, 73, 255],
                    PaintPath::FullPictureRebuild => [255, 112, 98, 255],
                    PaintPath::CanvasRedraw => [188, 136, 255, 255],
                };
                stroke[3] =
                    u8::try_from(u16::from(marker.frames) * 255 / u16::from(RAINBOW_FRAMES))
                        .unwrap_or(255);
                commands.push(outline(marker.rect, [0; 4], stroke));
                marker.frames -= 1;
            }
        }
        if let Some(rect) = highlight {
            commands.push(outline(rect, [56, 139, 253, 41], [56, 139, 253, 255]));
        }
        #[cfg(debug_assertions)]
        self.record_dev_error(&mut commands);
        commands
    }
}

fn outline(rect: Rect, color: [u8; 4], stroke_color: [u8; 4]) -> Command {
    Command::Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        corner_radius: 0.0,
        color,
        stroke_width: 1.5,
        stroke_color,
        gradient: None,
        shadow: None,
        box_shadows: Default::default(),
    }
}
