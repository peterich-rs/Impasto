//! One projection of a folded semantics tree. Platform adapters only map roles.

use std::collections::HashMap;

use uicore::{SemanticsId, SemanticsNode, SemanticsRole, SemanticsUpdate};

/// A node after Generic parents have been skipped, with scroll extent already resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectedNode {
    pub id: SemanticsId,
    pub parent: Option<SemanticsId>,
    pub rect: uicore::Rect,
    pub role: SemanticsRole,
    pub label: String,
    pub value: String,
    pub help: String,
    pub actions: uicore::SemanticsActions,
    pub flags: uicore::SemanticsFlags,
    pub selection: Option<(u32, u32)>,
    pub scroll_offset: (f32, f32),
    pub content: (f32, f32),
    pub scroll_forward: (f32, f32),
    pub scroll_backward: (f32, f32),
}

/// Drop Generic nodes. Scroll extent and step are computed once, here.
pub fn project(update: &SemanticsUpdate) -> Vec<ProjectedNode> {
    let by_id: HashMap<SemanticsId, &SemanticsNode> =
        update.nodes.iter().map(|node| (node.id, node)).collect();
    update
        .nodes
        .iter()
        .filter(|node| node.role != SemanticsRole::Generic)
        .map(|node| {
            let content = content_extent(update, node.id);
            let (forward, backward) = if node.role == SemanticsRole::Scroll
                || node.actions.contains(uicore::SemanticsActions::SCROLL)
            {
                (
                    scroll_step(node.rect.w, node.rect.h, content.0, content.1, true),
                    scroll_step(node.rect.w, node.rect.h, content.0, content.1, false),
                )
            } else {
                ((0.0, 0.0), (0.0, 0.0))
            };
            ProjectedNode {
                id: node.id,
                parent: resolved_parent(node, &by_id),
                rect: node.rect,
                role: node.role,
                label: node.label.clone(),
                value: node.value.clone().unwrap_or_default(),
                help: node.description.clone().unwrap_or_default(),
                actions: node.actions,
                flags: node.flags,
                selection: node
                    .selection
                    .or_else(|| node.caret.map(|unit| (unit, unit))),
                scroll_offset: node.scroll.unwrap_or((0.0, 0.0)),
                content,
                scroll_forward: forward,
                scroll_backward: backward,
            }
        })
        .collect()
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
    use super::{content_extent, project, scroll_step};
    use uicore::{
        SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsRole,
        SemanticsUpdate,
    };

    fn node(id: u64, parent: Option<u64>, role: SemanticsRole, w: f32, h: f32) -> SemanticsNode {
        SemanticsNode {
            id: SemanticsId(id),
            parent: parent.map(SemanticsId),
            rect: uicore::Rect {
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
        }
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
