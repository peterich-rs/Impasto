//! Accessibility snapshot mapping for the Android bridge.
//!
//! Generic nodes are dropped. Scroll increments use the same formula as macOS.

use uicore::{SemanticsActions, SemanticsRole};

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
    /// Scene-point box. The view converts it to screen pixels for TalkBack.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
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

/// Map already-projected nodes onto the Android bridge shape. Parent chain is not recomputed.
#[cfg(target_os = "android")]
pub fn bridge_nodes(nodes: &[hostshell::ProjectedNode]) -> Vec<BridgeNode> {
    nodes
        .iter()
        .map(|node| BridgeNode {
            id: node.id.0,
            parent: node.parent.map(|id| id.0),
            role: role_ordinal(node.role),
            label: node.label.clone(),
            value: node.value.clone(),
            flags: node.flags.0,
            actions: node.actions.0,
            offset_x: node.scroll_offset.0,
            offset_y: node.scroll_offset.1,
            content_w: node.content.0,
            content_h: node.content.1,
            description: node.help.clone(),
            x: node.rect.x,
            y: node.rect.y,
            w: node.rect.w,
            h: node.rect.h,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ROLE_SWITCH, ROLE_TEXT, ROLE_TEXT_FIELD, class_name, is_clickable, role_ordinal};
    use uicore::{SemanticsActions, SemanticsFlags, SemanticsId, SemanticsNode, SemanticsRole};

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
}
