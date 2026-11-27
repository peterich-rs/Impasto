//! One projection of a folded semantics tree. Platform adapters only map roles.

use std::collections::HashMap;

use engine::semantics::{
    SemanticsFlags, SemanticsId, SemanticsNode, SemanticsOp, SemanticsPatch, SemanticsRole,
    SemanticsUpdate,
};

/// A node after Generic parents have been skipped, with scroll extent already resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedNode {
    pub id: SemanticsId,
    pub parent: Option<SemanticsId>,
    pub rect: engine::Rect,
    pub role: SemanticsRole,
    pub label: String,
    pub value: String,
    pub help: String,
    pub autofill: Option<engine::semantics::AutofillField>,
    pub actions: engine::semantics::SemanticsActions,
    pub flags: SemanticsFlags,
    pub selection: Option<(u32, u32)>,
    pub scroll_offset: (f32, f32),
    pub content: (f32, f32),
    pub scroll_forward: (f32, f32),
    pub scroll_backward: (f32, f32),
    pub live: engine::semantics::LiveRegion,
}

/// Keeps the folded tree and its projection together. Text, selection and
/// focus patches keep the same parent resolution and scroll extents.
pub(crate) struct Projection {
    update: SemanticsUpdate,
    nodes: Vec<ProjectedNode>,
    platform_views: Option<bool>,
}

impl Projection {
    pub(crate) fn new(root: SemanticsId) -> Self {
        Self {
            update: SemanticsUpdate {
                root,
                nodes: Vec::new(),
            },
            nodes: Vec::new(),
            platform_views: None,
        }
    }

    pub(crate) fn apply(
        &mut self,
        patch: &SemanticsPatch,
        include_platform_views: bool,
    ) -> Option<&[ProjectedNode]> {
        let local = self.platform_views == Some(include_platform_views)
            && self.update.root == patch.root
            && patch.order.is_none()
            && patch.ops.iter().all(|op| {
                let SemanticsOp::Upsert(node) = op else {
                    return false;
                };
                self.update.nodes.iter().any(|old| {
                    old.id == node.id
                        && old.parent == node.parent
                        && old.role == node.role
                        && old.live == node.live
                        && old.scroll_content == node.scroll_content
                        && old.rect.w == node.rect.w
                        && old.rect.h == node.rect.h
                        && old.flags.contains(SemanticsFlags::PLATFORM_VIEW)
                            == node.flags.contains(SemanticsFlags::PLATFORM_VIEW)
                })
            });
        self.update.root = patch.root;
        self.platform_views = Some(include_platform_views);
        let mut changed = false;
        for op in &patch.ops {
            match op {
                SemanticsOp::Upsert(node) => {
                    if let Some(old) = self.update.nodes.iter_mut().find(|old| old.id == node.id) {
                        old.clone_from(node);
                    } else {
                        self.update.nodes.push(node.clone());
                    }
                    if local && let Some(old) = self.nodes.iter_mut().find(|old| old.id == node.id)
                    {
                        let next = project_node(node, old.parent, old.content);
                        changed |= *old != next;
                        *old = next;
                    }
                }
                SemanticsOp::Remove(id) => self.update.nodes.retain(|node| node.id != *id),
            }
        }
        if let Some(order) = &patch.order {
            let ranks: HashMap<_, _> = order.iter().copied().zip(0..).collect();
            self.update
                .nodes
                .sort_by_key(|node| ranks.get(&node.id).copied().unwrap_or(usize::MAX));
        }
        if !local {
            let nodes = project_with_platform_views(&self.update, include_platform_views);
            changed = self.nodes != nodes;
            self.nodes = nodes;
        }
        changed.then_some(&self.nodes)
    }
}

/// Drop Generic nodes. Scroll extent and step are computed once, here.
pub fn project(update: &SemanticsUpdate) -> Vec<ProjectedNode> {
    project_with_platform_views(update, false)
}

pub(crate) fn project_with_platform_views(
    update: &SemanticsUpdate,
    include_platform_views: bool,
) -> Vec<ProjectedNode> {
    let by_id: HashMap<SemanticsId, &SemanticsNode> =
        update.nodes.iter().map(|node| (node.id, node)).collect();
    let mut extents = HashMap::<SemanticsId, (f32, f32)>::new();
    for node in &update.nodes {
        if let Some(parent) = node.parent {
            let extent = extents.entry(parent).or_default();
            extent.0 = extent.0.max(node.rect.w);
            extent.1 = extent.1.max(node.rect.h);
        }
    }
    update
        .nodes
        .iter()
        .filter(|node| {
            node.role != SemanticsRole::Generic
                || node.live != engine::semantics::LiveRegion::Off
                || (include_platform_views && node.flags.contains(SemanticsFlags::PLATFORM_VIEW))
        })
        .map(|node| {
            let content = node
                .scroll_content
                .or_else(|| extents.get(&node.id).copied())
                .unwrap_or((node.rect.w, node.rect.h));
            project_node(node, resolved_parent(node, &by_id), content)
        })
        .collect()
}

fn project_node(
    node: &SemanticsNode,
    parent: Option<SemanticsId>,
    content: (f32, f32),
) -> ProjectedNode {
    let forward = if node.role == SemanticsRole::Scroll
        || node
            .actions
            .contains(engine::semantics::SemanticsActions::SCROLL)
    {
        scroll_step(node.rect.w, node.rect.h, content.0, content.1, true)
    } else {
        (0.0, 0.0)
    };
    ProjectedNode {
        id: node.id,
        parent,
        rect: node.rect,
        role: node.role,
        label: node.label.clone(),
        value: node.value.clone().unwrap_or_default(),
        help: node.description.clone().unwrap_or_default(),
        autofill: node.autofill.clone(),
        actions: node.actions,
        flags: node.flags,
        selection: node
            .selection
            .or_else(|| node.caret.map(|unit| (unit, unit))),
        scroll_offset: node.scroll.unwrap_or((0.0, 0.0)),
        content,
        scroll_forward: forward,
        scroll_backward: (-forward.0, -forward.1),
        live: node.live,
    }
}

fn resolved_parent(
    node: &SemanticsNode,
    by_id: &HashMap<SemanticsId, &SemanticsNode>,
) -> Option<SemanticsId> {
    let mut current = node.parent;
    while let Some(id) = current {
        match by_id.get(&id) {
            Some(parent) if parent.role != SemanticsRole::Generic => return Some(id),
            Some(parent) => current = parent.parent,
            None => return None,
        }
    }
    None
}

/// Direct children of `scroll_id`. No children → the viewport size.
pub fn content_extent(update: &SemanticsUpdate, scroll_id: SemanticsId) -> (f32, f32) {
    let Some(scroll) = update.nodes.iter().find(|node| node.id == scroll_id) else {
        return (0.0, 0.0);
    };
    let mut width = 0.0f32;
    let mut height = 0.0f32;
    let mut any = false;
    for child in &update.nodes {
        if child.parent == Some(scroll_id) {
            any = true;
            width = width.max(child.rect.w);
            height = height.max(child.rect.h);
        }
    }
    if any {
        (width, height)
    } else {
        (scroll.rect.w, scroll.rect.h)
    }
}

/// Forward increment is positive. Vertical overflow wins even when both axes overflow.
pub fn scroll_step(
    viewport_w: f32,
    viewport_h: f32,
    content_w: f32,
    content_h: f32,
    forward: bool,
) -> (f32, f32) {
    let (dx, dy) = if content_h > viewport_h {
        (0.0, viewport_h)
    } else if content_w > viewport_w {
        (viewport_w, 0.0)
    } else {
        (0.0, 0.0)
    };
    if forward { (dx, dy) } else { (-dx, -dy) }
}

#[cfg(test)]
mod tests {
    use super::{Projection, content_extent, project, scroll_step};
    use engine::semantics::{
        SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsOp, SemanticsPatch,
        SemanticsRole, SemanticsUpdate,
    };

    fn node(id: u64, parent: Option<u64>, role: SemanticsRole, w: f32, h: f32) -> SemanticsNode {
        SemanticsNode {
            id: SemanticsId(id),
            parent: parent.map(SemanticsId),
            rect: engine::Rect {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
            role,
            label: String::new(),
            value: None,
            actions: SemanticsActions(0),
            flags: SemanticsFlags(0),
            scroll: None,
            selection: None,
            caret: None,
            description: None,
            autofill: None,
            live: engine::semantics::LiveRegion::Off,
            scroll_content: None,
        }
    }

    #[test]
    fn projection_reuses_parent_and_extent_for_an_edit_and_rebuilds_after_removal() {
        let root = SemanticsId(1);
        let mut state = Projection::new(root);
        let scroll = node(1, None, SemanticsRole::Scroll, 100.0, 40.0);
        let mut field = node(2, Some(1), SemanticsRole::TextField, 100.0, 80.0);
        let mut patch = SemanticsPatch {
            root,
            ops: vec![
                SemanticsOp::Upsert(scroll),
                SemanticsOp::Upsert(field.clone()),
            ],
            order: None,
        };
        assert_eq!(
            state.apply(&patch, false).expect("initial nodes")[0].content,
            (100.0, 80.0)
        );
        field.value = Some("edited".into());
        field.selection = Some((2, 4));
        patch.ops = vec![SemanticsOp::Upsert(field)];
        let edited = state.apply(&patch, false).expect("edited nodes");
        assert_eq!(edited[0].content, (100.0, 80.0));
        assert_eq!(edited[1].parent, Some(root));
        assert_eq!(edited[1].value, "edited");
        assert_eq!(edited[1].selection, Some((2, 4)));
        assert!(state.apply(&patch, false).is_none());
        patch.ops = vec![SemanticsOp::Remove(SemanticsId(2))];
        let removed = state.apply(&patch, false).expect("updated tree");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].content, (100.0, 40.0));
    }

    #[test]
    fn generic_parents_are_skipped_and_vertical_overflow_wins() {
        let update = SemanticsUpdate {
            root: SemanticsId(1),
            nodes: vec![
                node(1, None, SemanticsRole::Generic, 100.0, 100.0),
                node(2, Some(1), SemanticsRole::Scroll, 100.0, 40.0),
                node(3, Some(2), SemanticsRole::Button, 100.0, 80.0),
            ],
        };
        let projected = project(&update);
        assert_eq!(projected.len(), 2);
        let button = projected
            .iter()
            .find(|node| node.role == SemanticsRole::Button)
            .unwrap();
        assert_eq!(button.parent, Some(SemanticsId(2)));
        assert_eq!(content_extent(&update, SemanticsId(2)), (100.0, 80.0));
        assert_eq!(scroll_step(100.0, 40.0, 100.0, 80.0, true), (0.0, 40.0));
        assert_eq!(
            projected
                .iter()
                .find(|node| node.id == SemanticsId(2))
                .unwrap()
                .scroll_forward,
            (0.0, 40.0)
        );
    }
}
