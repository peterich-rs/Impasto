//! Walk a laid-out tree into a semantics update.

use super::node::Kind;
use super::{NodeId, Rect, Tree};
use crate::{
    SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsOp, SemanticsPatch,
    SemanticsRole, SemanticsUpdate,
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
            caret: self.focused_caret(id),
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
        if node.is_focusable() && !disabled {
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
        let scroll = (node.kind == Kind::Scroll).then_some(node.scroll_offset());
        let kind = node.kind;
        let text = node
            .text()
            .map(|text| text.utf8.as_ref().to_owned())
            .unwrap_or_default();
        let (w, h) = (node.w, node.h);
        let first = node.first_child;
        let _ = node;
        let (x, y) = self.layout_point(id).ok()?;
        let rect = Rect { x, y, w, h };
        Some(Snapshot {
            kind,
            role,
            hint_label: hint.as_ref().and_then(|hint| hint.label.clone()),
            value: hint.as_ref().and_then(|hint| hint.value.clone()),
            selection: hint.as_ref().and_then(|hint| hint.selection),
            description: hint.as_ref().and_then(|hint| hint.description.clone()),
            text,
            rect,
            actions,
            flags,
            scroll,
            children: self.child_ids_from(first),
        })
    }

    fn focused_caret(&self, id: NodeId) -> Option<u32> {
        if self.focused != Some(id) {
            return None;
        }
        Some(
            self.node(id)
                .ok()?
                .text()?
                .editor
                .as_ref()?
                .value()
                .selection
                .extent,
        )
    }

    fn child_ids_from(&self, mut child: Option<NodeId>) -> Vec<NodeId> {
        let mut ids = Vec::new();
        while let Some(current) = child {
            ids.push(current);
            child = self.get(current).and_then(|node| node.next);
        }
        ids
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

struct BuiltSemantics {
    id: SemanticsId,
    kind: Kind,
    rect: Rect,
    children: Vec<NodeId>,
    node: SemanticsNode,
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
        self.mark_semantics_chain(id);
        Ok(())
    }

    pub(super) fn mark_semantics_chain(&mut self, mut id: NodeId) {
        loop {
            let parent = {
                let Ok(node) = self.node_mut(id) else {
                    return;
                };
                node.flags.insert(super::node::NodeFlags::SEMANTICS_DIRTY);
                node.parent
            };
            let Some(next) = parent else {
                return;
            };
            id = next;
        }
    }

    /// Diff against the previous patch. Clean nodes keep their strings and only
    /// refresh `rect` and scroll. `None` means nothing changed.
    pub fn semantics_patch(&mut self) -> Option<SemanticsPatch> {
        let root = self.root?;
        if !self.laid_out {
            return None;
        }
        let mut current = Vec::new();
        self.collect_semantics(root, None, &mut current);
        let mut live = std::collections::HashSet::new();
        let mut ops = Vec::new();
        for node in &current {
            live.insert(node.id);
            if self.semantics_cache.get(&node.id) != Some(node) {
                ops.push(SemanticsOp::Upsert(node.clone()));
            }
        }
        let stale: Vec<_> = self
            .semantics_cache
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in stale {
            self.semantics_cache.remove(&id);
            ops.push(SemanticsOp::Remove(id));
        }
        for node in &current {
            self.semantics_cache.insert(node.id, node.clone());
        }
        if ops.is_empty() {
            return None;
        }
        Some(SemanticsPatch {
            root: SemanticsId::from_node(root),
            ops,
        })
    }

    fn collect_semantics(
        &mut self,
        id: NodeId,
        parent: Option<SemanticsId>,
        out: &mut Vec<SemanticsNode>,
    ) {
        let Some(node) = self.semantics_node(id, parent) else {
            return;
        };
        let children = node.children.clone();
        let skip_children = node.kind == Kind::Clip && (node.rect.w == 0.0 || node.rect.h == 0.0);
        out.push(node.node);
        if skip_children {
            return;
        }
        let id = node.id;
        for child in children {
            self.collect_semantics(child, Some(id), out);
        }
    }

    fn semantics_node(
        &mut self,
        id: NodeId,
        parent: Option<SemanticsId>,
    ) -> Option<BuiltSemantics> {
        let sid = SemanticsId::from_node(id);
        let dirty = self
            .get(id)?
            .flags
            .contains(super::node::NodeFlags::SEMANTICS_DIRTY);
        if dirty && let Ok(node) = self.node_mut(id) {
            node.flags.remove(super::node::NodeFlags::SEMANTICS_DIRTY);
        }
        let snapshot = self.semantics_snapshot(id)?;
        let cached = (!dirty)
            .then(|| self.semantics_cache.get(&sid).cloned())
            .flatten();
        let label = if let Some(cached) = &cached {
            cached.label.clone()
        } else {
            snapshot.hint_label.clone().unwrap_or_else(|| {
                if snapshot.kind == Kind::PointerRegion {
                    self.descendant_text(id)
                } else {
                    snapshot.text.clone()
                }
            })
        };
        let value = cached
            .as_ref()
            .map(|cached| cached.value.clone())
            .unwrap_or_else(|| snapshot.value.clone());
        let description = cached
            .as_ref()
            .map(|cached| cached.description.clone())
            .unwrap_or_else(|| snapshot.description.clone());
        Some(BuiltSemantics {
            id: sid,
            kind: snapshot.kind,
            rect: snapshot.rect,
            children: snapshot.children,
            node: SemanticsNode {
                id: sid,
                parent,
                rect: snapshot.rect,
                role: snapshot.role,
                label,
                value,
                actions: snapshot.actions,
                flags: snapshot.flags,
                scroll: snapshot.scroll,
                selection: snapshot.selection,
                caret: self.focused_caret(id),
                description,
            },
        })
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
