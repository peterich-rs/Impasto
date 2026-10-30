//! Accessibility snapshot mapping for the Android bridge.
//!
//! Generic nodes are dropped. Scroll increments use the same formula as macOS.

use std::collections::HashMap;

use uicore::{SemanticsActions, SemanticsId, SemanticsNode, SemanticsRole, SemanticsUpdate};

pub const ROLE_GENERIC: i32 = 0;
pub const ROLE_BUTTON: i32 = 1;
pub const ROLE_TEXT: i32 = 2;
pub const ROLE_IMAGE: i32 = 3;
pub const ROLE_SCROLL: i32 = 4;
pub const ROLE_SWITCH: i32 = 5;
pub const ROLE_TEXT_FIELD: i32 = 6;
pub const ROLE_SEARCH_FIELD: i32 = 7;
pub const ROLE_TEXT_AREA: i32 = 8;

/// One node the accessibility bridge exposes. Generic nodes never appear.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeNode {
    pub id: u64,
    pub parent: Option<u64>,
    pub role: i32,
    pub label: String,
    pub value: String,
    pub flags: u32,
    pub actions: u32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub content_w: f32,
    pub content_h: f32,
    pub description: String,
}

pub fn role_ordinal(role: SemanticsRole) -> i32 {
    match role {
        SemanticsRole::Generic => ROLE_GENERIC,
        SemanticsRole::Button => ROLE_BUTTON,
        SemanticsRole::Text => ROLE_TEXT,
        SemanticsRole::Image => ROLE_IMAGE,
        SemanticsRole::Scroll => ROLE_SCROLL,
        SemanticsRole::Switch => ROLE_SWITCH,
        SemanticsRole::TextField => ROLE_TEXT_FIELD,
        SemanticsRole::SearchField => ROLE_SEARCH_FIELD,
        SemanticsRole::TextArea => ROLE_TEXT_AREA,
    }
}

pub fn class_name(role: i32) -> &'static str {
    match role {
        ROLE_BUTTON => "android.widget.Button",
        ROLE_SWITCH => "android.widget.Switch",
        ROLE_TEXT_FIELD | ROLE_SEARCH_FIELD | ROLE_TEXT_AREA => "android.widget.EditText",
        ROLE_SCROLL => "android.widget.ScrollView",
        ROLE_IMAGE => "android.widget.Image",
        ROLE_TEXT => "android.widget.TextView",
        _ => "android.view.View",
    }
}

pub fn is_clickable(actions: u32) -> bool {
    SemanticsActions(actions).contains(SemanticsActions::ACTIVATE)
}

/// Drop Generic nodes and hang their children on the nearest non-Generic ancestor.
pub fn bridge_nodes(update: &SemanticsUpdate) -> Vec<BridgeNode> {
    let by_id: HashMap<SemanticsId, &SemanticsNode> =
        update.nodes.iter().map(|node| (node.id, node)).collect();
    update
        .nodes
        .iter()
        .filter(|node| node.role != SemanticsRole::Generic)
        .map(|node| {
            let (offset_x, offset_y, content_w, content_h) = if node.role == SemanticsRole::Scroll {
                let (content_w, content_h) = content_extent(update, node.id);
                let (offset_x, offset_y) = node.scroll.unwrap_or((0.0, 0.0));
                (offset_x, offset_y, content_w, content_h)
            } else {
                (0.0, 0.0, 0.0, 0.0)
            };
            BridgeNode {
                id: node.id.0,
                parent: resolved_parent(node, &by_id).map(|id| id.0),
                role: role_ordinal(node.role),
                label: node.label.clone(),
                value: node.value.clone().unwrap_or_default(),
                flags: node.flags.0,
                actions: node.actions.0,
                offset_x,
                offset_y,
                content_w,
                content_h,
                description: node.description.clone().unwrap_or_default(),
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

/// Direct children of `scroll_id` in the original snapshot. No children → viewport.
pub fn content_extent(update: &SemanticsUpdate, scroll_id: SemanticsId) -> (f32, f32) {
    let Some(scroll) = update.nodes.iter().find(|node| node.id == scroll_id) else {
        return (0.0, 0.0);
    };
    let mut width: f32 = 0.0;
    let mut height: f32 = 0.0;
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

/// Scroll delta for `id`. Missing node or no overflow → `(0, 0)`.
pub fn scroll_action(update: &SemanticsUpdate, id: SemanticsId, forward: bool) -> (f32, f32) {
    let Some(node) = update.nodes.iter().find(|node| node.id == id) else {
        return (0.0, 0.0);
    };
    if node.role != SemanticsRole::Scroll && !node.actions.contains(SemanticsActions::SCROLL) {
        return (0.0, 0.0);
    }
    let (content_w, content_h) = content_extent(update, id);
    scroll_step(node.rect.w, node.rect.h, content_w, content_h, forward)
}

#[cfg(test)]
mod tests {
    use super::{
        ROLE_BUTTON, ROLE_SWITCH, ROLE_TEXT, ROLE_TEXT_FIELD, bridge_nodes, class_name,
        is_clickable, role_ordinal, scroll_action, scroll_step,
    };
    use uicore::{
        SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsRole,
        SemanticsUpdate,
    };

    fn rect(w: f32, h: f32) -> uicore::Rect {
        uicore::Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
        }
    }

    fn node(id: u64, parent: Option<u64>, role: SemanticsRole, label: &str) -> SemanticsNode {
        SemanticsNode {
            id: SemanticsId(id),
            parent: parent.map(SemanticsId),
            rect: rect(40.0, 20.0),
            role,
            label: label.to_owned(),
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
    fn switch_and_text_field_ordinals_are_locked() {
        assert_eq!(role_ordinal(SemanticsRole::Switch), ROLE_SWITCH);
        assert_eq!(role_ordinal(SemanticsRole::Switch), 5);
        assert_eq!(role_ordinal(SemanticsRole::TextField), ROLE_TEXT_FIELD);
        assert_eq!(role_ordinal(SemanticsRole::TextField), 6);
        assert_eq!(class_name(ROLE_SWITCH), "android.widget.Switch");
        assert_eq!(class_name(ROLE_TEXT_FIELD), "android.widget.EditText");
    }

    #[test]
    fn a_text_node_is_not_clickable() {
        let text = node(2, Some(1), SemanticsRole::Text, "Count");
        assert_eq!(role_ordinal(text.role), ROLE_TEXT);
        assert!(!is_clickable(text.actions.0));
        let mut button = node(3, Some(1), SemanticsRole::Button, "Minus");
        button.actions = SemanticsActions(SemanticsActions::ACTIVATE);
        assert!(is_clickable(button.actions.0));
    }

    #[test]
    fn vertical_overflow_scrolls_by_the_viewport_height() {
        let (dx, dy) = scroll_step(200.0, 100.0, 200.0, 250.0, true);
        assert_eq!((dx, dy), (0.0, 100.0));
        let update = SemanticsUpdate {
            nodes: vec![
                SemanticsNode {
                    id: SemanticsId(1),
                    parent: None,
                    rect: rect(200.0, 100.0),
                    role: SemanticsRole::Scroll,
                    label: String::new(),
                    value: None,
                    actions: SemanticsActions(SemanticsActions::SCROLL),
                    flags: SemanticsFlags(0),
                    scroll: Some((0.0, 0.0)),
                    selection: None,
                    caret: None,
                    description: None,
                },
                SemanticsNode {
                    id: SemanticsId(2),
                    parent: Some(SemanticsId(1)),
                    rect: rect(200.0, 250.0),
                    role: SemanticsRole::Generic,
                    label: String::new(),
                    value: None,
                    actions: SemanticsActions(0),
                    flags: SemanticsFlags(0),
                    scroll: None,
                    selection: None,
                    caret: None,
                    description: None,
                },
            ],
            root: SemanticsId(1),
        };
        assert_eq!(scroll_action(&update, SemanticsId(1), true), (0.0, 100.0));
        assert_eq!(scroll_action(&update, SemanticsId(1), false), (0.0, -100.0));
    }

    #[test]
    fn both_axes_overflow_still_scroll_vertically() {
        assert_eq!(scroll_step(100.0, 100.0, 250.0, 250.0, true), (0.0, 100.0));
    }

    #[test]
    fn no_overflow_keeps_the_offset() {
        assert_eq!(scroll_step(100.0, 100.0, 100.0, 100.0, true), (0.0, 0.0));
        assert_eq!(scroll_step(100.0, 100.0, 80.0, 80.0, true), (0.0, 0.0));
        let update = SemanticsUpdate {
            nodes: vec![SemanticsNode {
                id: SemanticsId(1),
                parent: None,
                rect: rect(100.0, 100.0),
                role: SemanticsRole::Scroll,
                label: String::new(),
                value: None,
                actions: SemanticsActions(SemanticsActions::SCROLL),
                flags: SemanticsFlags(0),
                scroll: Some((12.0, 8.0)),
                selection: None,
                caret: None,
                description: None,
            }],
            root: SemanticsId(1),
        };
        assert_eq!(scroll_action(&update, SemanticsId(1), true), (0.0, 0.0));
    }

    #[test]
    fn generic_parent_is_dropped_and_the_button_hangs_on_the_root() {
        let mut button = node(2, Some(1), SemanticsRole::Button, "Go");
        button.actions = SemanticsActions(SemanticsActions::ACTIVATE);
        let update = SemanticsUpdate {
            nodes: vec![node(1, None, SemanticsRole::Generic, "wrap"), button],
            root: SemanticsId(1),
        };
        let nodes = bridge_nodes(&update);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].role, ROLE_BUTTON);
        assert_eq!(nodes[0].label, "Go");
        assert_eq!(nodes[0].parent, None);
        assert!(nodes.iter().all(|node| node.role != super::ROLE_GENERIC));
        assert!(is_clickable(nodes[0].actions));
    }
}
