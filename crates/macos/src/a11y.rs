//! Accessibility tree for the macOS host. Generic nodes are dropped; AppKit
//! objects are created on the main thread only.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use hostshell::{Command, Poster};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibilityButtonRole, NSAccessibilityCheckBoxRole, NSAccessibilityImageRole,
    NSAccessibilityRole, NSAccessibilityScrollAreaRole, NSAccessibilityStaticTextRole,
    NSAccessibilityTextFieldRole, NSView,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRange, NSRect, NSString};

/// One non-Generic node after Generic ancestors have been skipped.
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleNode {
    pub id: uicore::SemanticsId,
    pub parent: Option<uicore::SemanticsId>,
    pub rect: uicore::Rect,
    pub role: uicore::SemanticsRole,
    pub label: String,
    pub help: String,
    pub actions: uicore::SemanticsActions,
    pub flags: uicore::SemanticsFlags,
    /// Selection, or a collapsed caret. Empty when the node has neither.
    pub selection: Option<(u32, u32)>,
}

/// AppKit role that a visible node maps to. Switch is CheckBox.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxRole {
    Button,
    StaticText,
    Image,
    ScrollArea,
    CheckBox,
    TextField,
    SearchField,
    TextArea,
}

/// Stable slot for one [`uicore::SemanticsId`]. Disappeared ids are dropped.
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElementCache {
    slots: HashMap<u64, u64>,
    next: u64,
}

#[cfg(test)]
impl ElementCache {
    /// Reuse the slot for each still-live id. New ids get a fresh token.
    pub fn sync(&mut self, ids: &[u64]) -> Vec<u64> {
        let live: HashSet<u64> = ids.iter().copied().collect();
        self.slots.retain(|id, _| live.contains(id));
        let mut tokens = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(&token) = self.slots.get(&id) {
                tokens.push(token);
                continue;
            }
            self.next += 1;
            let token = self.next;
            self.slots.insert(id, token);
            tokens.push(token);
        }
        tokens
    }
}

/// Drop Generic nodes and hang their children on the nearest non-Generic ancestor.
pub fn visible_nodes(update: &uicore::SemanticsUpdate) -> Vec<VisibleNode> {
    let by_id: HashMap<uicore::SemanticsId, &uicore::SemanticsNode> =
        update.nodes.iter().map(|node| (node.id, node)).collect();
    update
        .nodes
        .iter()
        .filter(|node| node.role != uicore::SemanticsRole::Generic)
        .map(|node| VisibleNode {
            id: node.id,
            parent: resolved_parent(node, &by_id),
            rect: node.rect,
            role: node.role,
            label: node.label.clone(),
            help: node.description.clone().unwrap_or_default(),
            actions: node.actions,
            flags: node.flags,
            selection: node
                .selection
                .or_else(|| node.caret.map(|unit| (unit, unit))),
        })
        .collect()
}

fn resolved_parent(
    node: &uicore::SemanticsNode,
    by_id: &HashMap<uicore::SemanticsId, &uicore::SemanticsNode>,
) -> Option<uicore::SemanticsId> {
    let mut current = node.parent;
    while let Some(id) = current {
        match by_id.get(&id) {
            Some(parent) if parent.role != uicore::SemanticsRole::Generic => return Some(id),
            Some(parent) => current = parent.parent,
            None => return None,
        }
    }
    None
}

/// Button, Text, Image, TextField, Switch→CheckBox, Scroll. Generic is not an element.
#[cfg(test)]
pub fn ax_role(role: uicore::SemanticsRole) -> Option<AxRole> {
    match role {
        uicore::SemanticsRole::Generic => None,
        uicore::SemanticsRole::Button => Some(AxRole::Button),
        uicore::SemanticsRole::Text => Some(AxRole::StaticText),
        uicore::SemanticsRole::Image => Some(AxRole::Image),
        uicore::SemanticsRole::Scroll => Some(AxRole::ScrollArea),
        uicore::SemanticsRole::Switch => Some(AxRole::CheckBox),
        uicore::SemanticsRole::TextField => Some(AxRole::TextField),
        uicore::SemanticsRole::SearchField => Some(AxRole::SearchField),
        uicore::SemanticsRole::TextArea => Some(AxRole::TextArea),
    }
}

/// `1` when `CHECKED` is set, otherwise `0`.
pub fn checked_value(flags: uicore::SemanticsFlags) -> i32 {
    i32::from(flags.contains(uicore::SemanticsFlags::CHECKED))
}

/// Forward increment is positive. Vertical overflow wins even when both axes overflow.
#[cfg(test)]
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

/// Direct children of `scroll_id` in the original snapshot. No children → viewport.
#[cfg(test)]
pub fn content_extent(
    update: &uicore::SemanticsUpdate,
    scroll_id: uicore::SemanticsId,
) -> (f32, f32) {
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

pub fn scene_rect_to_screen(view: &NSView, rect: uicore::Rect) -> NSRect {
    let height = view.bounds().size.height;
    let local = NSRect::new(
        NSPoint::new(rect.x as f64, height - rect.y as f64 - rect.h as f64),
        CGSize::new(rect.w as f64, rect.h as f64),
    );
    let in_window = view.convertRect_toView(local, None);
    match view.window() {
        Some(window) => window.convertRectToScreen(in_window),
        None => in_window,
    }
}

pub(crate) struct A11yIvars {
    id: uicore::SemanticsId,
    poster: Poster,
    host: Cell<*const NSView>,
    parent: Cell<*const AnyObject>,
    label: RefCell<String>,
    help: RefCell<String>,
    role: Cell<uicore::SemanticsRole>,
    rect: Cell<uicore::Rect>,
    actions: Cell<u32>,
    flags: Cell<u32>,
    selected: Cell<Option<(u32, u32)>>,
    children: RefCell<Vec<Retained<A11yNode>>>,
}

define_class!(
    #[unsafe(super(objc2_app_kit::NSAccessibilityElement))]
    #[thread_kind = MainThreadOnly]
    #[ivars = A11yIvars]
    pub(crate) struct A11yNode;

    unsafe impl NSObjectProtocol for A11yNode {}

    impl A11yNode {
        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Retained<NSString> {
            NSString::from_str(&self.ivars().label.borrow())
        }

        #[unsafe(method_id(accessibilityHelp))]
        fn accessibility_help(&self) -> Retained<NSString> {
            NSString::from_str(&self.ivars().help.borrow())
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSAccessibilityRole>> {
            // SAFETY: AppKit role constants are immortal NSString objects.
            unsafe {
                match self.ivars().role.get() {
                    uicore::SemanticsRole::Button => Some(NSAccessibilityButtonRole.retain()),
                    uicore::SemanticsRole::Text => Some(NSAccessibilityStaticTextRole.retain()),
                    uicore::SemanticsRole::Image => Some(NSAccessibilityImageRole.retain()),
                    uicore::SemanticsRole::Scroll => Some(NSAccessibilityScrollAreaRole.retain()),
                    uicore::SemanticsRole::Switch => Some(NSAccessibilityCheckBoxRole.retain()),
                    uicore::SemanticsRole::TextField
                    | uicore::SemanticsRole::SearchField
                    | uicore::SemanticsRole::TextArea => {
                        // objc2 does not export the search-field or text-area role symbols.
                        Some(NSAccessibilityTextFieldRole.retain())
                    }
                    uicore::SemanticsRole::Generic => None,
                }
            }
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSNumber>> {
            match self.ivars().role.get() {
                uicore::SemanticsRole::Switch => Some(NSNumber::numberWithInt(checked_value(
                    uicore::SemanticsFlags(self.ivars().flags.get()),
                ))),
                _ => None,
            }
        }

        #[unsafe(method(accessibilitySelectedTextRange))]
        fn accessibility_selected_text_range(&self) -> NSRange {
            match self.ivars().selected.get() {
                Some((start, end)) => {
                    let (start, end) = if start <= end { (start, end) } else { (end, start) };
                    NSRange::new(start as usize, (end - start) as usize)
                }
                None => NSRange::new(usize::MAX, 0),
            }
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect {
            let host = self.ivars().host.get();
            if host.is_null() {
                return NSRect::new(NSPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0));
            }
            // SAFETY: `host` is the content view, retained for the window lifetime.
            scene_rect_to_screen(unsafe { &*host }, self.ivars().rect.get())
        }

        #[unsafe(method_id(accessibilityParent))]
        fn accessibility_parent(&self) -> Option<Retained<AnyObject>> {
            let parent = self.ivars().parent.get();
            if parent.is_null() {
                None
            } else {
                // SAFETY: parent is the host view or another live A11yNode.
                Some(unsafe { &*parent }.retain())
            }
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Option<Retained<NSArray<A11yNode>>> {
            let children = self.ivars().children.borrow();
            Some(NSArray::from_retained_slice(children.as_slice()))
        }

        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            let allowed = uicore::SemanticsActions(self.ivars().actions.get())
                .contains(uicore::SemanticsActions::ACTIVATE);
            if allowed {
                self.ivars()
                    .poster
                    .post(Command::SemanticsAction(uicore::SemanticsAction::Activate(
                        self.ivars().id,
                    )));
            }
            allowed
        }
    }
);

impl A11yNode {
    fn new(
        mtm: MainThreadMarker,
        id: uicore::SemanticsId,
        poster: Poster,
        host: *const NSView,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(A11yIvars {
            id,
            poster,
            host: Cell::new(host),
            parent: Cell::new(std::ptr::null()),
            label: RefCell::new(String::new()),
            help: RefCell::new(String::new()),
            role: Cell::new(uicore::SemanticsRole::Generic),
            rect: Cell::new(uicore::Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            }),
            actions: Cell::new(0),
            flags: Cell::new(0),
            selected: Cell::new(None),
            children: RefCell::new(Vec::new()),
        });
        // SAFETY: `init` is NSAccessibilityElement's designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    fn apply(&self, node: &VisibleNode) {
        *self.ivars().label.borrow_mut() = node.label.clone();
        *self.ivars().help.borrow_mut() = node.help.clone();
        self.ivars().role.set(node.role);
        self.ivars().rect.set(node.rect);
        self.ivars().actions.set(node.actions.0);
        self.ivars().flags.set(node.flags.0);
        self.ivars().selected.set(node.selection);
    }

    fn set_tree(&self, parent: *const AnyObject, children: Vec<Retained<A11yNode>>) {
        self.ivars().parent.set(parent);
        *self.ivars().children.borrow_mut() = children;
    }
}

/// Main-thread cache of AppKit elements, keyed by [`uicore::SemanticsId`].
pub(crate) struct A11yHost {
    elements: RefCell<HashMap<u64, Retained<A11yNode>>>,
    roots: RefCell<Vec<Retained<A11yNode>>>,
}

impl A11yHost {
    pub(crate) fn new() -> Self {
        Self {
            elements: RefCell::new(HashMap::new()),
            roots: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn apply(
        &self,
        mtm: MainThreadMarker,
        poster: &Poster,
        host: *const NSView,
        update: &uicore::SemanticsUpdate,
    ) {
        let visible = visible_nodes(update);
        let live: HashSet<u64> = visible.iter().map(|node| node.id.0).collect();
        {
            let mut elements = self.elements.borrow_mut();
            elements.retain(|id, _| live.contains(id));
            for node in &visible {
                if !elements.contains_key(&node.id.0) {
                    let created = A11yNode::new(mtm, node.id, poster.clone(), host);
                    elements.insert(node.id.0, created);
                }
                if let Some(element) = elements.get(&node.id.0) {
                    element.apply(node);
                }
            }
        }
        let elements = self.elements.borrow();
        let mut roots = Vec::new();
        for node in &visible {
            let children: Vec<Retained<A11yNode>> = visible
                .iter()
                .filter(|child| child.parent == Some(node.id))
                .filter_map(|child| elements.get(&child.id.0).cloned())
                .collect();
            let parent = match node.parent {
                None => host.cast::<AnyObject>(),
                Some(id) => elements
                    .get(&id.0)
                    .map(|parent| {
                        let ptr: *const A11yNode = Retained::as_ptr(parent);
                        ptr.cast::<AnyObject>()
                    })
                    .unwrap_or(host.cast::<AnyObject>()),
            };
            if let Some(element) = elements.get(&node.id.0) {
                element.set_tree(parent, children);
            }
            if node.parent.is_none()
                && let Some(element) = elements.get(&node.id.0)
            {
                roots.push(element.clone());
            }
        }
        drop(elements);
        *self.roots.borrow_mut() = roots;
    }

    pub(crate) fn root_children(&self) -> Retained<NSArray<A11yNode>> {
        let roots = self.roots.borrow();
        NSArray::from_retained_slice(roots.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AxRole, ElementCache, ax_role, checked_value, content_extent, scroll_step, visible_nodes,
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
    fn two_buttons_keep_their_labels() {
        let update = SemanticsUpdate {
            nodes: vec![
                node(1, None, SemanticsRole::Generic, ""),
                node(2, Some(1), SemanticsRole::Button, "Minus"),
                node(3, Some(1), SemanticsRole::Button, "Plus"),
            ],
            root: SemanticsId(1),
        };
        let visible = visible_nodes(&update);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].label, "Minus");
        assert_eq!(visible[1].label, "Plus");
        assert!(visible.iter().all(|node| node.parent.is_none()));
    }

    #[test]
    fn generic_between_buttons_is_not_an_element() {
        let update = SemanticsUpdate {
            nodes: vec![
                node(1, None, SemanticsRole::Generic, "root"),
                node(2, Some(1), SemanticsRole::Button, "Minus"),
                node(3, Some(1), SemanticsRole::Generic, "skip"),
                node(4, Some(3), SemanticsRole::Button, "Plus"),
            ],
            root: SemanticsId(1),
        };
        let visible = visible_nodes(&update);
        assert_eq!(visible.len(), 2);
        assert!(
            visible
                .iter()
                .all(|node| node.role == SemanticsRole::Button && node.parent.is_none())
        );
        assert!(visible.iter().all(|node| node.label != "skip"));
    }

    #[test]
    fn same_id_reuses_the_same_slot() {
        let mut cache = ElementCache::default();
        let first = cache.sync(&[10, 11]);
        let second = cache.sync(&[10, 11]);
        assert_eq!(first, second);
        let minus_only = cache.sync(&[10]);
        assert_eq!(minus_only, vec![first[0]]);
        let again = cache.sync(&[10, 11]);
        assert_eq!(again[0], first[0]);
        assert_ne!(again[1], first[1]);
    }

    #[test]
    fn switch_is_checkbox_and_checked_value_is_one() {
        assert_eq!(ax_role(SemanticsRole::Switch), Some(AxRole::CheckBox));
        assert_eq!(checked_value(SemanticsFlags(SemanticsFlags::CHECKED)), 1);
        assert_eq!(checked_value(SemanticsFlags(0)), 0);
    }

    #[test]
    fn description_is_help_and_stays_out_of_the_label() {
        let mut field = node(2, Some(1), SemanticsRole::TextField, "Name");
        field.description = Some("Required".to_owned());
        let update = SemanticsUpdate {
            nodes: vec![node(1, None, SemanticsRole::Generic, ""), field],
            root: SemanticsId(1),
        };
        let visible = visible_nodes(&update);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].label, "Name");
        assert_eq!(visible[0].help, "Required");
        assert!(!visible[0].label.contains("Required"));
    }

    #[test]
    fn vertical_overflow_uses_viewport_height() {
        assert_eq!(scroll_step(200.0, 100.0, 200.0, 250.0, true), (0.0, 100.0));
        assert_eq!(scroll_step(100.0, 100.0, 250.0, 250.0, true), (0.0, 100.0));
        assert_eq!(scroll_step(100.0, 100.0, 100.0, 100.0, true), (0.0, 0.0));
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
        assert_eq!(content_extent(&update, SemanticsId(1)), (200.0, 250.0));
    }

    #[test]
    fn press_requires_activate() {
        let mut button = node(2, None, SemanticsRole::Button, "Go");
        button.actions = SemanticsActions(SemanticsActions::ACTIVATE);
        let text = node(3, None, SemanticsRole::Text, "Hi");
        assert!(button.actions.contains(SemanticsActions::ACTIVATE));
        assert!(!text.actions.contains(SemanticsActions::ACTIVATE));
    }
}
