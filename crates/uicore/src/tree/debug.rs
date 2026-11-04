//! Developer-only layout diagnostics. Everything here is gated behind
//! [`Tree::set_debug_scan`] and costs nothing when the flag is off.

use super::node::Kind;
use super::{NodeId, Tree};

/// Which main axis an overflow was measured on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DebugAxis {
    Horizontal,
    Vertical,
}

impl DebugAxis {
    pub fn name(self) -> &'static str {
        match self {
            Self::Horizontal => "horizontal",
            Self::Vertical => "vertical",
        }
    }
}

/// One overflow found by the post-layout scan.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutIssue {
    pub parent: NodeId,
    pub axis: DebugAxis,
    /// Container content-box size on that axis.
    pub parent_size: f32,
    /// Extent the children actually occupy.
    pub used: f32,
    /// `used - parent_size`. Positive means overflow.
    pub excess: f32,
    /// The child that extends furthest.
    pub worst_child: NodeId,
}

/// Debug-only state on [`Tree`]. All fields default off/empty.
#[derive(Default)]
pub(crate) struct DebugState {
    pub(crate) scan_overflow: bool,
    pub(crate) issues: Vec<LayoutIssue>,
    pub(crate) new_issues: Vec<LayoutIssue>,
    pub(crate) seen_issues: Vec<(NodeId, DebugAxis)>,
    /// Pointer tracing for the devtools inspector.
    pub(crate) pointer_trace: bool,
    pub(crate) suppress_dispatch: bool,
    pub(crate) last_pointer: Option<(f32, f32, &'static str)>,
    /// Subtrees owned by the devtool layer itself; probes skip them.
    pub(crate) debug_owned: Vec<NodeId>,
    /// Scratch for the scan walk.
    pub(crate) walk_stack: Vec<WalkFrame>,
}

pub(crate) struct WalkFrame {
    tolerant: bool,
    next: Option<NodeId>,
}

/// Half a point of slop, matching the hit-test threshold.
const EPSILON: f32 = 0.5;

impl Tree {
    /// Turn the post-layout overflow scan on or off. Off by default.
    pub fn set_debug_scan(&mut self, on: bool) {
        self.debug.scan_overflow = on;
        if !on {
            self.debug.issues.clear();
            self.debug.new_issues.clear();
            self.debug.seen_issues.clear();
        }
    }

    /// True when the scan runs after each layout.
    pub fn debug_scan_enabled(&self) -> bool {
        self.debug.scan_overflow
    }

    /// Report Row/Column children that exceed the container on the main axis.
    /// Scroll viewports, Text with `Visible` overflow, and Clip subtrees are
    /// intentional overflow and are skipped.
    pub(crate) fn scan_overflow(&mut self) {
        let Some(root) = self.root else {
            return;
        };
        let mut found: Vec<LayoutIssue> = Vec::new();
        let mut stack = std::mem::take(&mut self.debug.walk_stack);
        stack.clear();
        stack.push(WalkFrame {
            tolerant: false,
            next: self.node(root).map(|n| n.first_child).unwrap_or(None),
        });
        while let Some(frame) = stack.last_mut() {
            let Some(child_id) = frame.next else {
                stack.pop();
                continue;
            };
            let child = match self.node(child_id) {
                Ok(node) => node,
                Err(_) => {
                    frame.next = None;
                    continue;
                }
            };
            frame.next = child.next;
            let kind = child.kind;
            let tolerant = frame.tolerant || self.subtree_is_tolerant(child_id, kind);
            if !tolerant && let Some(issue) = self.main_axis_overflow(child_id, kind) {
                found.push(issue);
            }
            stack.push(WalkFrame {
                tolerant,
                next: child.first_child,
            });
        }
        self.debug.walk_stack = stack;
        // The root itself is a layout container too.
        if let Some(root) = self.root
            && let Ok(node) = self.node(root)
        {
            let kind = node.kind;
            if !self.subtree_is_tolerant(root, kind)
                && let Some(issue) = self.main_axis_overflow(root, kind)
            {
                found.push(issue);
            }
        }
        let seen = &mut self.debug.seen_issues;
        let fresh: Vec<LayoutIssue> = found
            .iter()
            .filter(|issue| !seen.contains(&(issue.parent, issue.axis)))
            .copied()
            .collect();
        self.debug.seen_issues = found.iter().map(|i| (i.parent, i.axis)).collect();
        self.debug.issues = found;
        self.debug.new_issues.extend(fresh);
    }

    /// Whether overflow inside this node is by design.
    fn subtree_is_tolerant(&self, id: NodeId, kind: Kind) -> bool {
        match kind {
            Kind::Scroll | Kind::Clip => true,
            Kind::Text => self
                .node(id)
                .ok()
                .and_then(|node| node.text())
                .is_some_and(|text| text.style.overflow == protocol::TextOverflow::Visible),
            _ => false,
        }
    }

    /// The overflow of one Row/Column, if any.
    fn main_axis_overflow(&self, id: NodeId, kind: Kind) -> Option<LayoutIssue> {
        let axis = match kind {
            Kind::Row => DebugAxis::Horizontal,
            Kind::Column => DebugAxis::Vertical,
            _ => return None,
        };
        let parent = self.node(id).ok()?;
        let (limit, used) = match axis {
            DebugAxis::Horizontal => (parent.w, self.extent(id, true)),
            DebugAxis::Vertical => (parent.h, self.extent(id, false)),
        };
        let excess = used - limit;
        if excess <= EPSILON {
            return None;
        }
        Some(LayoutIssue {
            parent: id,
            axis,
            parent_size: limit,
            used,
            excess,
            worst_child: self.worst_child(id, axis),
        })
    }

    /// How far the children reach on the axis, measured from the parent box
    /// start in parent-relative coordinates.
    fn extent(&self, id: NodeId, horizontal: bool) -> f32 {
        let mut used = 0.0f32;
        let mut child = self.node(id).ok().and_then(|n| n.first_child);
        while let Some(cid) = child {
            if let Ok(node) = self.node(cid) {
                let edge = if horizontal {
                    node.x + node.w
                } else {
                    node.y + node.h
                };
                used = used.max(edge.max(0.0));
                child = node.next;
            } else {
                break;
            }
        }
        used
    }

    fn worst_child(&self, id: NodeId, axis: DebugAxis) -> NodeId {
        let mut worst = id;
        let mut worst_edge = f32::MIN;
        let mut child = self.node(id).ok().and_then(|n| n.first_child);
        while let Some(cid) = child {
            if let Ok(node) = self.node(cid) {
                let edge = match axis {
                    DebugAxis::Horizontal => node.x + node.w,
                    DebugAxis::Vertical => node.y + node.h,
                };
                if edge > worst_edge {
                    worst_edge = edge;
                    worst = cid;
                }
                child = node.next;
            } else {
                break;
            }
        }
        worst
    }

    /// Take issues that appeared since the last take. Empty when the scan is
    /// off or nothing new happened.
    pub fn take_layout_issues(&mut self) -> Vec<LayoutIssue> {
        std::mem::take(&mut self.debug.new_issues)
    }

    /// The full current report, for the devtools layout page.
    pub fn layout_issues(&self) -> &[LayoutIssue] {
        &self.debug.issues
    }

    /// JSON object `{"issues":[...]}` for the layout page and `frame.json`.
    pub fn layout_issues_json(&self) -> String {
        let mut out = String::from("{\"issues\":[");
        for (index, issue) in self.debug.issues.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"parent\":{},\"axis\":\"{}\",\"parentSize\":{:.2},\"used\":{:.2},\"excess\":{:.2},\"worstChild\":{}}}",
                issue.parent.to_raw(),
                issue.axis.name(),
                issue.parent_size,
                issue.used,
                issue.excess,
                issue.worst_child.to_raw()
            ));
        }
        out.push_str("]}");
        out
    }

    /// Mark a subtree as debug-owned: probes and snapshots skip it.
    pub fn set_debug_owned(&mut self, id: NodeId, on: bool) {
        if on {
            if !self.debug.debug_owned.contains(&id) {
                self.debug.debug_owned.push(id);
            }
        } else {
            self.debug.debug_owned.retain(|other| *other != id);
        }
    }

    /// Whether `id` sits inside any debug-owned subtree.
    pub(crate) fn is_debug_owned(&self, id: NodeId) -> bool {
        let mut current = Some(id);
        while let Some(node) = current {
            if self.debug.debug_owned.contains(&node) {
                return true;
            }
            current = self.node(node).ok().and_then(|n| n.parent);
        }
        false
    }

    /// Record pointer events for `debugLastPointer`. When suppress is set,
    /// listeners do not fire; the app stays inert while the inspector selects.
    pub fn set_pointer_trace(&mut self, on: bool, suppress: bool) {
        self.debug.pointer_trace = on;
        self.debug.suppress_dispatch = suppress;
    }

    /// The last pointer event as `(x, y, phase)` in scene points, if tracing.
    pub(crate) fn note_pointer(&mut self, x: f32, y: f32, phase: &'static str) {
        if self.debug.pointer_trace {
            self.debug.last_pointer = Some((x, y, phase));
        }
    }

    /// `[x, y, "phase"]` for the devtools inspector, or `null`.
    pub fn debug_last_pointer(&self) -> Option<(f32, f32, &'static str)> {
        self.debug.last_pointer
    }

    /// The render tree with scene rects as JSON, for the inspector page.
    /// Debug-owned subtrees are cut.
    pub fn debug_snapshot(&self) -> String {
        let mut out = String::from("{\"root\":");
        match self.root {
            Some(root) => self.write_snapshot_node(root, &mut out),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }

    fn write_snapshot_node(&self, id: NodeId, out: &mut String) {
        let Ok(node) = self.node(id) else {
            out.push_str("null");
            return;
        };
        let rect = self.rect(id).unwrap_or(super::Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        out.push_str("{\"id\":");
        push_u64(out, id.to_raw());
        out.push_str(",\"kind\":\"");
        out.push_str(kind_name(node.kind));
        out.push_str("\",\"x\":");
        push_f32(out, rect.x);
        out.push_str(",\"y\":");
        push_f32(out, rect.y);
        out.push_str(",\"w\":");
        push_f32(out, rect.w);
        out.push_str(",\"h\":");
        push_f32(out, rect.h);
        let focusable = node.flags.contains(super::node::NodeFlags::FOCUSABLE);
        out.push_str(",\"focusable\":");
        out.push_str(if focusable { "true" } else { "false" });
        if let Some(text) = node.text() {
            out.push_str(",\"text\":\"");
            let prefix: String = text.utf8.chars().take(80).collect();
            push_escaped(out, &prefix);
            out.push('"');
        }
        out.push_str(",\"debugOwned\":");
        out.push_str(if self.debug.debug_owned.contains(&id) {
            "true"
        } else {
            "false"
        });
        out.push_str(",\"children\":[");
        let mut child = node.first_child;
        let mut first = true;
        while let Some(cid) = child {
            let Ok(child_node) = self.node(cid) else {
                break;
            };
            if !first {
                out.push(',');
            }
            first = false;
            if self.is_debug_owned(cid) {
                // Skip the devtool layer itself but keep walking siblings.
                out.push_str("null");
            } else {
                self.write_snapshot_node(cid, out);
            }
            child = child_node.next;
        }
        out.push_str("]}");
    }

    /// Hit-test chain for a scene point, root to leaf, skipping debug-owned
    /// subtrees. Same geometry the pointer path uses (last frame).
    pub fn debug_probe(&self, x: f32, y: f32) -> String {
        let mut out = String::from("{\"point\":{\"x\":");
        push_f32(&mut out, x);
        out.push_str(",\"y\":");
        push_f32(&mut out, y);
        out.push_str("},\"chain\":[");
        let mut chain: Vec<NodeId> = Vec::new();
        if let Some(root) = self.root {
            self.probe_walk(root, x, y, &mut chain);
        }
        let mut first = true;
        for id in chain {
            let Ok(node) = self.node(id) else {
                continue;
            };
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str("{\"id\":");
            push_u64(&mut out, id.to_raw());
            out.push_str(",\"kind\":\"");
            out.push_str(kind_name(node.kind));
            out.push_str("\"}");
        }
        out.push_str("]}");
        out
    }

    /// Depth-first hit chain, pushing every containing ancestor.
    fn probe_walk(&self, id: NodeId, x: f32, y: f32, chain: &mut Vec<NodeId>) -> bool {
        if self.is_debug_owned(id) {
            return false;
        }
        let Ok(node) = self.node(id) else {
            return false;
        };
        let Ok(rect) = self.rect(id) else {
            return false;
        };
        if x < rect.x || y < rect.y || x > rect.x + rect.w || y > rect.y + rect.h {
            return false;
        }
        chain.push(id);
        // Children are tested front-to-back (last child first). The first
        // branch that takes the point is the innermost hit.
        let mut child = node.last_child;
        while let Some(cid) = child {
            if self.probe_walk(cid, x, y, chain) {
                return true;
            }
            child = self.node(cid).ok().and_then(|n| n.prev);
        }
        true
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Row => "row",
        Kind::Column => "column",
        Kind::Grid => "grid",
        Kind::Padding => "padding",
        Kind::Align => "align",
        Kind::Sized => "sized",
        Kind::Stack => "stack",
        Kind::Clip => "clip",
        Kind::Opacity => "opacity",
        Kind::Scroll => "scroll",
        Kind::Transform => "transform",
        Kind::Text => "text",
        Kind::Image => "image",
        Kind::PointerRegion => "pointer",
        Kind::Decorated => "decorated",
    }
}

fn push_u64(out: &mut String, value: u64) {
    out.push_str(&value.to_string());
}

fn push_f32(out: &mut String, value: f32) {
    if value.is_finite() {
        out.push_str(&format!("{value:.2}"));
    } else {
        out.push('0');
    }
}

fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}
