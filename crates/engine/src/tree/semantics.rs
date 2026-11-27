//! Walk a laid-out tree into a semantics update.

use super::node::Kind;
use super::{NodeId, Rect, Tree};
use crate::semantics::{
    SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsOp, SemanticsPatch,
    SemanticsRole, SemanticsUpdate,
};

impl Tree {
    #[cfg(feature = "js")]
    pub(crate) fn semantics_pending(&self) -> bool {
        self.semantics_enabled && (self.semantics_full || !self.scroll_semantics.is_empty())
    }

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
            if snapshot.pointer {
                self.descendant_text(id)
            } else {
                snapshot.text
            }
        });
        let password = snapshot.flags.contains(SemanticsFlags::PASSWORD);
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
            caret: (!password).then(|| self.focused_caret(id)).flatten(),
            description: snapshot.description,
            autofill: snapshot.autofill,
            live: snapshot.live,
            scroll_content: snapshot.scroll_content,
        });
        if password || (snapshot.clip && (snapshot.rect.w == 0.0 || snapshot.rect.h == 0.0)) {
            return;
        }
        for child in snapshot.children {
            self.walk_semantics(child, Some(SemanticsId::from_node(id)), out);
        }
    }

    fn semantics_snapshot(&self, id: NodeId) -> Option<Snapshot> {
        let node = self.node(id).ok()?;
        if node.flags.intersects(
            super::node::NodeFlags::CULL_PAINT
                | super::node::NodeFlags::HIDDEN
                | super::node::NodeFlags::SEMANTICS_HIDDEN,
        ) {
            return None;
        }
        let hint = node.semantics.as_ref();
        let role = hint.and_then(|hint| hint.role).unwrap_or(match node.kind {
            Kind::Text => SemanticsRole::Text,
            Kind::Image => SemanticsRole::Image,
            Kind::Scroll => SemanticsRole::Scroll,
            Kind::View | Kind::Stack => SemanticsRole::Generic,
        });
        let disabled = hint.is_some_and(|hint| hint.disabled == Some(true));
        let read_only = hint.is_some_and(|hint| hint.read_only == Some(true));
        let mut actions = SemanticsActions(0);
        if node.is_pointer() && !disabled {
            actions.0 |= SemanticsActions::ACTIVATE;
        }
        if node.is_scroll() {
            actions.0 |= SemanticsActions::SCROLL;
        }
        let mut flags = SemanticsFlags(0);
        if self.platform_views.contains_key(&id) {
            flags.0 |= SemanticsFlags::PLATFORM_VIEW;
        }
        if node.is_focusable() && !disabled {
            flags.0 |= SemanticsFlags::FOCUSABLE;
        }
        if self.focused == Some(id) {
            flags.0 |= SemanticsFlags::FOCUSED;
        }
        if disabled {
            flags.0 |= SemanticsFlags::DISABLED;
        }
        let password = hint.is_some_and(|hint| hint.password);
        if password {
            flags.0 |= SemanticsFlags::PASSWORD;
        }
        if hint.is_some_and(|hint| hint.checked == Some(true)) {
            flags.0 |= SemanticsFlags::CHECKED;
        }
        if read_only {
            flags.0 |= SemanticsFlags::READ_ONLY;
        } else if !disabled && hint.is_some_and(|hint| hint.editable == Some(true)) {
            flags.0 |= SemanticsFlags::EDITABLE;
        }
        if flags.contains(SemanticsFlags::EDITABLE) {
            actions.0 |= SemanticsActions::SET_TEXT;
            if !password && hint.is_some_and(|hint| hint.selection.is_some()) {
                actions.0 |= SemanticsActions::SET_SELECTION;
            }
        }
        let scroll = node.is_scroll().then_some(node.scroll_offset());
        let live = hint
            .and_then(|hint| hint.live)
            .unwrap_or(crate::semantics::LiveRegion::Off);
        let scroll_content = node.is_scroll().then(|| self.scroll_content(id)).flatten();
        let kind = node.kind;
        let pointer = node.is_pointer();
        let clip = node.role.contains(super::node::Role::CLIP);
        let text = node
            .text()
            .map(|text| text.utf8.as_ref().to_owned())
            .unwrap_or_default();
        let autofill = hint
            .filter(|_| self.autofill_enabled && flags.contains(SemanticsFlags::EDITABLE))
            .and_then(|hint| hint.autofill.clone());
        let (w, h) = (node.w, node.h);
        let first = node.first_child;
        let _ = node;
        let (x, y) = self.layout_point(id).ok()?;
        let rect = self.visible_scene_rect(id).unwrap_or(Rect { x, y, w, h });
        Some(Snapshot {
            kind,
            pointer,
            clip,
            role,
            hint_label: hint.and_then(|hint| hint.label.clone()),
            value: hint
                .filter(|_| !password)
                .and_then(|hint| hint.value.clone()),
            selection: hint.filter(|_| !password).and_then(|hint| hint.selection),
            description: hint.and_then(|hint| hint.description.clone()),
            autofill,
            live,
            scroll_content,
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
    clip: bool,
    rect: Rect,
    children: Vec<NodeId>,
    node: SemanticsNode,
}

struct Snapshot {
    kind: Kind,
    pointer: bool,
    clip: bool,
    role: SemanticsRole,
    text: String,
    hint_label: Option<String>,
    value: Option<String>,
    selection: Option<(u32, u32)>,
    description: Option<String>,
    autofill: Option<crate::semantics::AutofillField>,
    live: crate::semantics::LiveRegion,
    scroll_content: Option<(f32, f32)>,
    rect: Rect,
    actions: SemanticsActions,
    flags: SemanticsFlags,
    scroll: Option<(f32, f32)>,
    children: Vec<NodeId>,
}

impl Tree {
    pub fn set_semantics_hidden(&mut self, id: NodeId, hidden: bool) -> Result<(), crate::Error> {
        use super::node::NodeFlags;

        let node = self.node_mut(id)?;
        if node.flags.contains(NodeFlags::SEMANTICS_HIDDEN) != hidden {
            node.flags.set(NodeFlags::SEMANTICS_HIDDEN, hidden);
            self.mark_semantics_chain(id);
        }
        Ok(())
    }

    pub fn set_semantics(
        &mut self,
        id: NodeId,
        hint: crate::semantics::SemanticsHint,
    ) -> Result<(), crate::Error> {
        let supports_autofill = hint.autofill.is_some();
        self.node_mut(id)?.semantics = Some(hint);
        if supports_autofill {
            self.autofill_nodes.insert(id);
        } else {
            self.autofill_nodes.remove(&id);
        }
        self.invalidate_style_deps(crate::style::DepMask::DISABLED);
        self.mark_semantics_chain(id);
        Ok(())
    }

    pub(super) fn mark_semantics_chain(&mut self, mut id: NodeId) {
        if !self.publish_semantics() {
            return;
        }
        self.semantics_full = true;
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

    pub fn semantics_enabled(&self) -> bool {
        self.semantics_enabled
    }

    pub fn autofill_enabled(&self) -> bool {
        self.autofill_enabled
    }

    pub fn set_autofill_enabled(&mut self, enabled: bool) {
        if self.autofill_enabled != enabled {
            self.autofill_enabled = enabled;
            self.semantics_full = true;
            self.semantics_cache.clear();
        }
    }

    pub(super) fn publish_semantics(&self) -> bool {
        self.semantics_enabled || (self.autofill_enabled && !self.autofill_nodes.is_empty())
    }

    /// Hosts with an on-demand accessibility tree can suspend its snapshots.
    pub fn set_semantics_enabled(&mut self, enabled: bool) {
        if self.semantics_enabled == enabled {
            return;
        }
        self.semantics_enabled = enabled;
        self.semantics_full = true;
        self.semantics_cache.clear();
        self.scroll_semantics.clear();
    }

    /// Diff against the previous patch. Clean nodes keep their strings and only
    /// refresh `rect` and scroll. `None` means nothing changed.
    pub fn semantics_patch(&mut self) -> Option<SemanticsPatch> {
        let root = self.root?;
        if !self.laid_out {
            return None;
        }
        if !self.publish_semantics() {
            if self.semantics_cache.is_empty() {
                return None;
            }
            let ops = self
                .semantics_cache
                .drain()
                .map(|(id, _)| SemanticsOp::Remove(id))
                .collect();
            self.semantics_order.clear();
            return Some(SemanticsPatch {
                root: SemanticsId::from_node(root),
                ops,
                order: Some(Vec::new()),
            });
        }
        if self.layout_passes != self.semantics_layout_seen || self.semantics_full {
            self.semantics_layout_seen = self.layout_passes;
            self.semantics_full = false;
            self.scroll_semantics.clear();
            return self.semantics_full_patch(root);
        }
        if self.scroll_semantics.is_empty() {
            return None;
        }
        if self
            .semantics_cache
            .values()
            .any(|node| node.autofill.is_some())
        {
            self.scroll_semantics.clear();
            return self.semantics_full_patch(root);
        }
        self.patch_scroll_offsets(root)
    }

    fn patch_scroll_offsets(&mut self, root: NodeId) -> Option<SemanticsPatch> {
        let ids = std::mem::take(&mut self.scroll_semantics);
        let mut seen = std::collections::HashSet::new();
        let mut ops = Vec::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let sid = SemanticsId::from_node(id);
            let Some(mut node) = self.semantics_cache.get(&sid).cloned() else {
                self.semantics_full = true;
                return self.semantics_full_patch(root);
            };
            node.scroll = self.node(id).ok().map(|node| node.scroll_offset());
            self.semantics_cache.insert(sid, node.clone());
            ops.push(SemanticsOp::Upsert(node));
        }
        if ops.is_empty() {
            None
        } else {
            Some(SemanticsPatch {
                root: SemanticsId::from_node(root),
                ops,
                order: None,
            })
        }
    }

    fn semantics_full_patch(&mut self, root: NodeId) -> Option<SemanticsPatch> {
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
        let order = current.iter().map(|node| node.id).collect::<Vec<_>>();
        let order = (order != self.semantics_order).then(|| {
            self.semantics_order.clone_from(&order);
            order
        });
        if ops.is_empty() && order.is_none() {
            return None;
        }
        Some(SemanticsPatch {
            root: SemanticsId::from_node(root),
            ops,
            order,
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
        let skip_children = node.node.flags.contains(SemanticsFlags::PASSWORD)
            || (node.clip && (node.rect.w == 0.0 || node.rect.h == 0.0));
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
                if snapshot.flags.contains(SemanticsFlags::PASSWORD) {
                    String::new()
                } else if snapshot.pointer {
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
            clip: snapshot.clip,
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
                caret: if snapshot.flags.contains(SemanticsFlags::PASSWORD) {
                    None
                } else {
                    self.focused_caret(id)
                },
                description,
                autofill: snapshot.autofill,
                live: snapshot.live,
                scroll_content: snapshot.scroll_content,
            },
        })
    }

    /// Turns an accessibility action into pointer or scroll updates.
    pub fn semantics_action(
        &mut self,
        action: crate::semantics::SemanticsAction,
    ) -> Vec<super::Gesture> {
        match action {
            crate::semantics::SemanticsAction::Focus(id) => self
                .lookup(id)
                .filter(|node| {
                    self.semantics_snapshot(*node)
                        .is_some_and(|snapshot| snapshot.flags.contains(SemanticsFlags::EDITABLE))
                })
                .and_then(|node| self.focus(node).ok())
                .unwrap_or_default(),
            crate::semantics::SemanticsAction::Activate(id) => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                let Ok(rect) = self.rect(node) else {
                    return Vec::new();
                };
                let x = rect.x + rect.w * 0.5;
                let y = rect.y + rect.h * 0.5;
                let mut gestures = self.pointer(crate::input::PointerEvent::new(
                    crate::input::Phase::Down,
                    x,
                    y,
                    0.0,
                ));
                gestures.extend(self.pointer(crate::input::PointerEvent::new(
                    crate::input::Phase::Up,
                    x,
                    y,
                    0.0,
                )));
                gestures
            }
            crate::semantics::SemanticsAction::Scroll { id, dx, dy } => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                let Ok((x, y)) = self.scroll_offset(node) else {
                    return Vec::new();
                };
                let _ = self.set_scroll_offset(node, x + dx, y + dy);
                Vec::new()
            }
            crate::semantics::SemanticsAction::SetText { id, text } => {
                self.replace_semantics_text(id, text, false)
            }
            crate::semantics::SemanticsAction::Autofill { id, text } => {
                self.replace_semantics_text(id, text, true)
            }
            crate::semantics::SemanticsAction::SetSelection { id, start, end } => {
                let Some(node) = self.lookup(id) else {
                    return Vec::new();
                };
                if self.focused != Some(node)
                    || !self.semantics_snapshot(node).is_some_and(|snapshot| {
                        snapshot.actions.contains(SemanticsActions::SET_SELECTION)
                    })
                {
                    return Vec::new();
                }
                if self
                    .text_edit(node, crate::text::Edit::SetSelection { start, end })
                    .is_err()
                {
                    return Vec::new();
                }
                vec![super::Gesture::TextReplaced(node)]
            }
        }
    }

    pub fn editor_autofill_hint(&self, node: NodeId) -> Option<&'static str> {
        self.get(node)?
            .semantics
            .as_ref()?
            .autofill
            .as_ref()
            .map(|field| field.hint)
    }

    /// Resolves the field again on the UI thread before a platform fill or edit.
    pub fn semantics_edit_target(&self, id: SemanticsId, autofill: bool) -> Option<NodeId> {
        let node = self.lookup(id)?;
        let snapshot = self.semantics_snapshot(node)?;
        if !snapshot.actions.contains(SemanticsActions::SET_TEXT)
            || (autofill
                && (snapshot.autofill.is_none()
                    || snapshot.rect.w <= 0.0
                    || snapshot.rect.h <= 0.0))
        {
            return None;
        }
        let mut current = Some(node);
        while let Some(ancestor) = current {
            let entry = self.node(ancestor).ok()?;
            if entry.flags.intersects(
                super::node::NodeFlags::CULL_PAINT
                    | super::node::NodeFlags::HIDDEN
                    | super::node::NodeFlags::SEMANTICS_HIDDEN,
            ) {
                return None;
            }
            if Some(ancestor) == self.root {
                return Some(node);
            }
            current = entry.parent;
        }
        None
    }

    fn replace_semantics_text(
        &mut self,
        id: SemanticsId,
        text: String,
        autofill: bool,
    ) -> Vec<super::Gesture> {
        let Some(node) = self.semantics_edit_target(id, autofill) else {
            return Vec::new();
        };
        if self.text_edit(node, crate::text::Edit::Load(text)).is_err() {
            return Vec::new();
        }
        vec![super::Gesture::TextReplaced(node)]
    }

    fn lookup(&self, id: SemanticsId) -> Option<NodeId> {
        let node = NodeId::from_raw(id.0);
        self.node(node).ok().map(|_| node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_view_slots_follow_scene_reading_order() -> Result<(), crate::Error> {
        let mut tree = Tree::new(crate::text::MonoShaper::new());
        let root = tree.column(0.0)?;
        let first = tree.text("first", 16.0, [0, 0, 0, 255])?;
        let native = tree.column(0.0)?;
        tree.set_platform_view(native, "editor")?;
        tree.add_child(root, first)?;
        tree.add_child(root, native)?;
        tree.layout()?;

        let patch = tree.semantics_patch().expect("initial semantics");
        let root_id = SemanticsId::from_node(root);
        let text_id = SemanticsId::from_node(first);
        let native_id = SemanticsId::from_node(native);
        assert_eq!(patch.order, Some(vec![root_id, text_id, native_id]));
        assert!(
            patch
                .ops
                .iter()
                .any(|op| matches!(op, SemanticsOp::Upsert(node)
            if node.id == native_id && node.flags.contains(SemanticsFlags::PLATFORM_VIEW)))
        );

        tree.insert(root, native, Some(first))?;
        tree.layout()?;
        assert_eq!(
            tree.semantics_patch().and_then(|patch| patch.order),
            Some(vec![root_id, native_id, text_id])
        );
        Ok(())
    }

    #[test]
    fn restoring_semantics_publishes_current_labels_without_another_layout()
    -> Result<(), crate::Error> {
        let mut tree = Tree::new(crate::text::MonoShaper::new());
        let text = tree.text("before", 16.0, [0, 0, 0, 255])?;
        tree.layout()?;
        assert!(tree.semantics_patch().is_some());

        tree.set_semantics_enabled(false);
        tree.set_text(text, "after")?;
        tree.layout()?;
        assert!(tree.semantics_patch().is_none());

        tree.set_semantics_enabled(true);
        let patch = tree.semantics_patch().expect("restored snapshot");
        assert!(patch.ops.iter().any(|op| matches!(
            op, SemanticsOp::Upsert(node) if node.id == SemanticsId::from_node(text) && node.label == "after"
        )));
        assert!(tree.semantics_patch().is_none());
        Ok(())
    }

    #[test]
    fn hidden_subtree_is_removed_from_the_incremental_snapshot() -> Result<(), crate::Error> {
        let mut tree = Tree::new(crate::text::MonoShaper::new());
        let root = tree.column(0.0)?;
        let panel = tree.column(0.0)?;
        let text = tree.text("hidden", 16.0, [0, 0, 0, 255])?;
        tree.add_child(root, panel)?;
        tree.add_child(panel, text)?;
        tree.layout()?;
        assert!(tree.semantics_patch().is_some());

        tree.node_mut(panel)?
            .flags
            .insert(crate::tree::node::NodeFlags::HIDDEN);
        tree.mark_semantics_chain(panel);
        let patch = tree
            .semantics_patch()
            .expect("removed subtrees are published");
        assert!(patch.ops.iter().any(
            |op| matches!(op, SemanticsOp::Remove(id) if *id == SemanticsId::from_node(text))
        ));
        Ok(())
    }
}
