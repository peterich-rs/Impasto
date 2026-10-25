//! Walk a laid-out tree into a semantics update.

use super::node::Kind;
use super::{NodeId, Rect, Tree};
use crate::{
    SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsRole, SemanticsUpdate,
};

impl Tree {
    /// One node per laid-out scene node. `None` before the first successful layout.
    pub fn semantics(&self) -> Option<SemanticsUpdate> {
        let root = self.root?;
        if !self.laid_out {
            return None;
        }
        let mut nodes = Vec::new();
        self.walk_semantics(root, None, &mut nodes);
        if nodes.is_empty() {
            return None;
        }
        Some(SemanticsUpdate {
            root: SemanticsId::from_node(root),
            nodes,
        })
    }

    fn walk_semantics(
        &self,
        id: NodeId,
        parent: Option<SemanticsId>,
        out: &mut Vec<SemanticsNode>,
    ) {
        let Some(snapshot) = self.semantics_snapshot(id) else {
            return;
        };
        let label = snapshot.hint_label.unwrap_or_else(|| {
            if snapshot.kind == Kind::PointerRegion {
                self.descendant_text(id)
            } else {
                snapshot.text
            }
        });
        out.push(SemanticsNode {
            id: SemanticsId::from_node(id),
            parent,
            rect: snapshot.rect,
            role: snapshot.role,
            label,
            value: snapshot.value,
            actions: snapshot.actions,
            flags: snapshot.flags,
            scroll: snapshot.scroll,
            selection: snapshot.selection,
            description: snapshot.description,
        });
        if snapshot.kind == Kind::Clip && (snapshot.rect.w == 0.0 || snapshot.rect.h == 0.0) {
            return;
        }
        for child in snapshot.children {
            self.walk_semantics(child, Some(SemanticsId::from_node(id)), out);
        }
    }

    fn semantics_snapshot(&self, id: NodeId) -> Option<Snapshot> {
        let node = self.node(id).ok()?;
        let hint = node.semantics.clone();
        let role = hint
            .as_ref()
            .and_then(|hint| hint.role)
            .unwrap_or(match node.kind {
                Kind::PointerRegion => SemanticsRole::Button,
                Kind::Text => SemanticsRole::Text,
                Kind::Image => SemanticsRole::Image,
                Kind::Scroll => SemanticsRole::Scroll,
                _ => SemanticsRole::Generic,
            });
        let disabled = hint
            .as_ref()
            .is_some_and(|hint| hint.disabled == Some(true));
        let read_only = hint
            .as_ref()
            .is_some_and(|hint| hint.read_only == Some(true));
        let mut actions = SemanticsActions(0);
        if node.kind == Kind::PointerRegion && !disabled {
            actions.0 |= SemanticsActions::ACTIVATE;
        }
        if node.kind == Kind::Scroll {
            actions.0 |= SemanticsActions::SCROLL;
        }
        let mut flags = SemanticsFlags(0);
        if node.focusable && !disabled {
            flags.0 |= SemanticsFlags::FOCUSABLE;
        }
        if disabled {
            flags.0 |= SemanticsFlags::DISABLED;
        }
        if hint.as_ref().is_some_and(|hint| hint.checked == Some(true)) {
            flags.0 |= SemanticsFlags::CHECKED;
        }
        if read_only {
            flags.0 |= SemanticsFlags::READ_ONLY;
        } else if !disabled
            && hint
                .as_ref()
                .is_some_and(|hint| hint.editable == Some(true))
        {
            flags.0 |= SemanticsFlags::EDITABLE;
        }
        if flags.contains(SemanticsFlags::EDITABLE) {
            actions.0 |= SemanticsActions::SET_TEXT;
        }
        let scroll = (node.kind == Kind::Scroll).then_some(node.scroll);
        Some(Snapshot {
            kind: node.kind,
            role,
            hint_label: hint.as_ref().and_then(|hint| hint.label.clone()),
            value: hint.as_ref().and_then(|hint| hint.value.clone()),
            selection: hint.as_ref().and_then(|hint| hint.selection),
            description: hint.as_ref().and_then(|hint| hint.description.clone()),
            text: node
                .text
                .as_ref()
                .map(|text| text.utf8.clone())
                .unwrap_or_default(),
            rect: Rect {
                x: node.x,
                y: node.y,
                w: node.w,
                h: node.h,
            },
            actions,
            flags,
            scroll,
            children: node.children.clone(),
        })
    }

    fn descendant_text(&self, id: NodeId) -> String {
        let Some(snapshot) = self.semantics_snapshot(id) else {
            return String::new();
        };
        if snapshot.kind == Kind::Text {
            return snapshot.text;
        }
        snapshot
            .children
            .into_iter()
            .map(|child| self.descendant_text(child))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

struct Snapshot {
    kind: Kind,
    role: SemanticsRole,
    text: String,
    hint_label: Option<String>,
    value: Option<String>,
    selection: Option<(u32, u32)>,
    description: Option<String>,
    rect: Rect,
    actions: SemanticsActions,
    flags: SemanticsFlags,
    scroll: Option<(f32, f32)>,
    children: Vec<NodeId>,
}

impl Tree {
    pub fn set_semantics(
        &mut self,
        id: NodeId,
        hint: crate::SemanticsHint,
    ) -> Result<(), crate::Error> {
        self.node_mut(id)?.semantics = Some(hint);
        Ok(())
    }

    /// Turns an accessibility action into pointer or scroll updates.
    pub fn semantics_action(&mut self, action: crate::SemanticsAction) -> Vec<super::Gesture> {
        match action {
            crate::SemanticsAction::Activate(id) => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                let Ok(rect) = self.rect(node) else {
                    return Vec::new();
                };
                let x = rect.x + rect.w * 0.5;
                let y = rect.y + rect.h * 0.5;
                let mut gestures =
                    self.pointer(crate::PointerEvent::new(crate::Phase::Down, x, y, 0.0));
                gestures.extend(self.pointer(crate::PointerEvent::new(
                    crate::Phase::Up,
                    x,
                    y,
                    0.0,
                )));
                gestures
            }
            crate::SemanticsAction::Scroll { id, dx, dy } => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                let Ok((x, y)) = self.scroll_offset(node) else {
                    return Vec::new();
                };
                let _ = self.set_scroll_offset(node, x + dx, y + dy);
                Vec::new()
            }
            crate::SemanticsAction::SetText { id, text } => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                if self.text_edit(node, crate::Edit::Load(text)).is_err() {
                    return Vec::new();
                }
                vec![super::Gesture::TextReplaced(node)]
            }
        }
    }

    fn lookup(&self, id: SemanticsId) -> Option<NodeId> {
        let node = NodeId::from_raw(id.0);
        self.node(node).ok().map(|_| node)
    }
}
