//! AppKit window. The render thread owns the engine and the session.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

use hostshell::{
    Boot, BootError, ClipboardHost, Command, Launch as ShellLaunch, Link, LinkingHost, LogLevel,
    NativePtr, PermissionsHost, Poster, Services, SessionError, StorageHost, SurfaceTarget,
    metrics,
};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, available, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSEvent, NSEventModifierFlags, NSFont, NSFontTextStyleBody, NSScreen, NSTextInputClient,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
    NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSArray, NSAttributedString, NSDictionary, NSDistributedNotificationCenter, NSLocale,
    NSNotFound, NSNotification, NSNotificationSuspensionBehavior, NSPoint, NSRange, NSRect,
    NSString, NSUInteger, NSURL,
};
use session::{InputEvent, Key};

use crate::text_client::Marked;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLPixelFormat};
use objc2_quartz_core::{CALayer, CAMetalLayer};

use crate::pointer::pointer_event;
use crate::surface::MetalSurface;
use crate::vsync::DisplayLink;
use uicore::{Button, Modifiers, Phase, PointerEvent};

const CONTENT_WIDTH: f64 = 360.0;
const CONTENT_HEIGHT: f64 = 640.0;
const FONT_FAMILY: &str = "Sans";

fn modifiers_of(event: &NSEvent) -> Modifiers {
    let flags = event.modifierFlags();
    let mut modifiers = Modifiers::empty();
    if flags.contains(NSEventModifierFlags::Shift) {
        modifiers = modifiers.with(Modifiers::SHIFT);
    }
    if flags.contains(NSEventModifierFlags::Control) {
        modifiers = modifiers.with(Modifiers::CONTROL);
    }
    if flags.contains(NSEventModifierFlags::Option) {
        modifiers = modifiers.with(Modifiers::ALT);
    }
    if flags.contains(NSEventModifierFlags::Command) {
        modifiers = modifiers.with(Modifiers::META);
    }
    modifiers
}

fn shortcut_character(event: &NSEvent) -> Option<char> {
    let text = event.charactersIgnoringModifiers()?;
    let value = text.to_string();
    let mut chars = value.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || ch.is_control() {
        return None;
    }
    Some(ch)
}

struct ContentIvars {
    poster: Poster,
    wants_ime: Cell<bool>,
    marked: RefCell<Marked>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
    a11y: crate::a11y::A11yHost,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ContentIvars]
    struct ContentView;

    unsafe impl NSObjectProtocol for ContentView {}

    #[allow(non_snake_case)]
    unsafe impl NSTextInputClient for ContentView {
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insertText_replacementRange(
            &self,
            string: &AnyObject,
            _replacement_range: NSRange,
        ) {
            self.insert_marked_text(string);
        }

        #[unsafe(method(doCommandBySelector:))]
        unsafe fn doCommandBySelector(&self, selector: Sel) {
            self.perform_command(selector);
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn setMarkedText_selectedRange_replacementRange(
            &self,
            string: &AnyObject,
            _selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            let Some(text) = object_string(string) else {
                return;
            };
            let events = self.ivars().marked.borrow_mut().mark(&text);
            self.post_edits(events);
        }

        #[unsafe(method(unmarkText))]
        fn unmarkText(&self) {
            let events = self.ivars().marked.borrow_mut().unmark();
            self.post_edits(events);
        }

        #[unsafe(method(selectedRange))]
        fn selectedRange(&self) -> NSRange {
            let marked = self.ivars().marked.borrow();
            NSRange {
                location: marked.selection.start as NSUInteger,
                length: marked
                    .selection
                    .end
                    .saturating_sub(marked.selection.start) as NSUInteger,
            }
        }

        #[unsafe(method(markedRange))]
        fn markedRange(&self) -> NSRange {
            match self.ivars().marked.borrow().marked_utf16() {
                Some((start, length)) => NSRange {
                    location: start as NSUInteger,
                    length: length as NSUInteger,
                },
                None => NSRange {
                    location: NSNotFound as NSUInteger,
                    length: 0,
                },
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn hasMarkedText(&self) -> bool {
            self.ivars().marked.borrow().has_marked()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributedSubstringForProposedRange_actualRange(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn validAttributesForMarkedText(
            &self,
        ) -> Retained<NSArray<objc2_foundation::NSAttributedStringKey>> {
            NSArray::from_slice(&[])
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn firstRectForCharacterRange_actualRange(
            &self,
            _range: NSRange,
            actual_range: *mut NSRange,
        ) -> NSRect {
            if !actual_range.is_null() {
                let location = self.ivars().marked.borrow().caret_utf16() as NSUInteger;
                // SAFETY: AppKit passes a writable range or null. The null case returned above.
                unsafe {
                    *actual_range = NSRange {
                        location,
                        length: 0,
                    };
                }
            }
            let caret = self.ivars().marked.borrow().caret;
            if caret.w > 0.0 && caret.h > 0.0 {
                self.scene_rect_to_screen(caret)
            } else {
                self.screen_bounds()
            }
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn characterIndexForPoint(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }

    impl ContentView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.window().is_some()
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            false
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Retained<NSArray<crate::a11y::A11yNode>> {
            self.ivars().a11y.root_children()
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let modifiers = modifiers_of(event);
            let chord = modifiers.contains(Modifiers::CONTROL)
                || modifiers.contains(Modifiers::ALT)
                || modifiers.contains(Modifiers::META);
            if chord && let Some(ch) = shortcut_character(event) {
                self.post_edits(vec![InputEvent::Key(session::KeyPress {
                    key: Key::Char(ch),
                    shift: modifiers.contains(Modifiers::SHIFT),
                    modifiers,
                })]);
                return;
            }
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &NSEvent) {
            let local = self.convertPoint_fromView(event.locationInWindow(), None);
            let (x, y) =
                crate::pointer::scene_point(local.x, local.y, self.bounds().size.height);
            self.ivars().poster.post(Command::Pointer(PointerEvent::zoom(
                x,
                y,
                event.timestamp() * 1000.0,
                event.magnification() as f32,
            )));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.enqueue(Phase::Down, event, Button::Primary);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.enqueue(Phase::Move, event, Button::Primary);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.enqueue(Phase::Up, event, Button::Primary);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.enqueue(Phase::Down, event, Button::Secondary);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.enqueue(Phase::Move, event, Button::Secondary);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.enqueue(Phase::Up, event, Button::Secondary);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.enqueue(Phase::Move, event, Button::None);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) {
            self.post_outside(event);
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe { msg_send![super(self), updateTrackingAreas] }
            self.install_tracking_area();
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.enqueue(Phase::Wheel, event, Button::None);
        }
    }
);

impl ContentView {
    fn new(mtm: MainThreadMarker, frame: CGRect, poster: Poster) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ContentIvars {
            poster,
            wants_ime: Cell::new(false),
            marked: RefCell::new(Marked::default()),
            tracking: RefCell::new(None),
            a11y: crate::a11y::A11yHost::new(),
        });
        // SAFETY: `initWithFrame:` is NSView's designated initializer. The ivars are
        // already installed, and `frame` is a finite content rectangle.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn install_tracking_area(&self) {
        if let Some(old) = self.ivars().tracking.borrow_mut().take() {
            self.removeTrackingArea(&old);
        }
        let options = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                self.bounds(),
                options,
                Some(self),
                None,
            )
        };
        self.addTrackingArea(&area);
        *self.ivars().tracking.borrow_mut() = Some(area);
    }

    fn post_outside(&self, event: &NSEvent) {
        self.ivars().poster.post(Command::Pointer(PointerEvent {
            pointer: uicore::PointerId(0),
            kind: uicore::PointerKind::Mouse,
            phase: Phase::Move,
            x: -1.0,
            y: -1.0,
            time_ms: event.timestamp() * 1000.0,
            button: Button::None,
            buttons: uicore::Buttons::empty(),
            modifiers: modifiers_of(event),
            dx: 0.0,
            dy: 0.0,
            zoom: 0.0,
        }));
    }

    fn enqueue(&self, phase: Phase, event: &NSEvent, button: Button) {
        let local = self.convertPoint_fromView(event.locationInWindow(), None);
        let height = self.bounds().size.height;
        let modifiers = modifiers_of(event);
        let (dx, dy) = if phase == Phase::Wheel {
            // AppKit's positive delta Y points up. Scene Y grows downward.
            (
                event.scrollingDeltaX() as f32,
                -(event.scrollingDeltaY() as f32),
            )
        } else {
            (0.0, 0.0)
        };
        self.ivars().poster.post(Command::Pointer(pointer_event(
            phase,
            local.x,
            local.y,
            height,
            event.timestamp() * 1000.0,
            button,
            modifiers,
            dx,
            dy,
        )));
    }

    fn post_edits(&self, events: Vec<InputEvent>) {
        for event in events {
            self.ivars().poster.post(Command::Input(event));
        }
    }

    fn insert_marked_text(&self, object: &AnyObject) {
        let Some(text) = object_string(object) else {
            return;
        };
        let events = self.ivars().marked.borrow_mut().insert(&text);
        self.post_edits(events);
    }

    fn perform_command(&self, selector: Sel) {
        let name = selector_name(selector);
        if let Some(press) = command_key(&name) {
            self.post_edits(vec![InputEvent::Key(press)]);
            return;
        }
        let events = match name.as_str() {
            "deleteBackward:" => self.ivars().marked.borrow_mut().backspace(),
            "copy:" => vec![InputEvent::Copy],
            "cut:" => vec![InputEvent::Cut],
            "paste:" => vec![InputEvent::Paste {
                plain: read_pasteboard(),
                html: read_pasteboard_html(),
            }],
            _ => {
                eprintln!("text command ignored: {name}");
                return;
            }
        };
        self.post_edits(events);
    }

    fn scene_rect_to_screen(&self, rect: uicore::Rect) -> NSRect {
        let height = self.bounds().size.height;
        let local = NSRect::new(
            NSPoint::new(rect.x as f64, height - rect.y as f64 - rect.h as f64),
            CGSize::new(rect.w as f64, rect.h as f64),
        );
        let in_window = self.convertRect_toView(local, None);
        match self.window() {
            Some(window) => window.convertRectToScreen(in_window),
            None => in_window,
        }
    }

    fn screen_bounds(&self) -> NSRect {
        let bounds = self.bounds();
        let in_window = self.convertRect_toView(bounds, None);
        match self.window() {
            Some(window) => window.convertRectToScreen(in_window),
            None => in_window,
        }
    }

    fn apply_semantics(&self, update: uicore::SemanticsUpdate) {
        self.ivars().a11y.apply(
            self.mtm(),
            &self.ivars().poster,
            self as *const ContentView as *const NSView,
            &update,
        );
    }

    fn apply_ime_update(&self, update: &session::ImeUpdate) {
        let Some(window) = self.window() else {
            return;
        };
        match update {
            session::ImeUpdate::Hide => {
                self.ivars().wants_ime.set(false);
                let _: bool = window.makeFirstResponder(None);
            }
            session::ImeUpdate::Show(state) | session::ImeUpdate::Update(state) => {
                self.ivars().marked.borrow_mut().apply(state);
                self.ivars().wants_ime.set(true);
                if matches!(update, session::ImeUpdate::Show(_)) {
                    let _: bool = window.makeFirstResponder(Some(self));
                }
            }
        }
    }
}

fn object_string(object: &AnyObject) -> Option<String> {
    if let Some(text) = object.downcast_ref::<NSString>() {
        return Some(text.to_string());
    }
    if let Some(text) = object.downcast_ref::<NSAttributedString>() {
        return Some(text.string().to_string());
    }
    None
}

fn selector_name(selector: Sel) -> String {
    selector.name().to_string_lossy().into_owned()
}

/// Trimmed launch URL. Empty is not a URL.
pub(crate) fn nonempty_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// Cold-start `openURLs:` that duplicates `--url`. Later opens always post.
fn skip_launch_duplicate(launched: bool, initial: Option<&str>, url: &str) -> bool {
    !launched && initial == Some(url)
}

struct HostState {
    window: Retained<NSWindow>,
    layer: Retained<CAMetalLayer>,
    link: Link,
    /// `--url` at process start. `set_launch_url` does not overwrite this copy.
    initial_url: Option<String>,
    launched: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostState]
    struct Host;

    unsafe impl NSObjectProtocol for Host {}

    unsafe impl NSApplicationDelegate for Host {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.ivars().launched.set(true);
            self.finish_launching();
        }

        #[unsafe(method(application:openURLs:))]
        fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            for index in 0..urls.count() {
                let Some(text) = urls.objectAtIndex(index).absoluteString() else {
                    continue;
                };
                let Some(url) = nonempty_url(&text.to_string()) else {
                    continue;
                };
                if skip_launch_duplicate(
                    self.ivars().launched.get(),
                    self.ivars().initial_url.as_deref(),
                    url.as_str(),
                ) {
                    continue;
                }
                self.ivars().link.post(Command::LaunchUrl(url));
                return;
            }
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Detached));
            let _ = self.ivars().link.shutdown_and_join();
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Resumed));
            self.post_settings();
        }

        #[unsafe(method(themeChanged:))]
        fn theme_changed(&self, _notification: &NSNotification) {
            self.post_settings();
        }

        #[unsafe(method(applicationWillResignActive:))]
        fn will_resign_active(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Inactive));
        }

        #[unsafe(method(applicationDidHide:))]
        fn did_hide(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Hidden));
        }

        #[unsafe(method(applicationDidReceiveMemoryWarning:))]
        fn did_receive_memory_warning(&self, _notification: &NSNotification) {
            self.ivars().link.post(Command::MemoryWarning);
        }
    }

    unsafe impl NSWindowDelegate for Host {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            let _ = self.ivars().link.shutdown_and_join();
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            self.post_metrics();
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing(&self, _notification: &NSNotification) {
            self.post_metrics();
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            self.ivars().link.post(Command::WindowFocus(true));
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            self.ivars().link.post(Command::WindowFocus(false));
        }

        #[unsafe(method(windowDidMiniaturize:))]
        fn window_did_miniaturize(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Hidden));
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn window_did_deminiaturize(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(session::Lifecycle::Resumed));
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion(&self, _notification: &NSNotification) {
            let visible = self
                .ivars()
                .window
                .occlusionState()
                .contains(NSWindowOcclusionState::Visible);
            if visible {
                self.ivars().link.post(Command::Invalidate);
            }
        }
    }
);

impl Host {
    fn new(mtm: MainThreadMarker, state: HostState) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: `init` is NSObject's designated initializer, and the ivars are already installed.
        unsafe { msg_send![super(this), init] }
    }

    fn finish_launching(&self) {
        let window = &self.ivars().window;
        window.center();
        window.makeKeyAndOrderFront(None);
        let app = NSApplication::sharedApplication(self.mtm());
        if available!(macos = 14.0) {
            app.activate();
        } else {
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
        }
        let scale = point_scale(window, self.mtm());
        let layer = Retained::as_ptr(&self.ivars().layer) as *mut c_void;
        self.ivars().link.post(Command::SurfaceReady(SurfaceTarget {
            id: hostshell::SurfaceId::fresh(),
            layer: NativePtr(layer),
            metrics: metrics(CONTENT_WIDTH as f32, CONTENT_HEIGHT as f32, scale as f32),
        }));
        self.post_settings();
        println!("host ready");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    fn post_metrics(&self) {
        let window = &self.ivars().window;
        let scale = point_scale(window, self.mtm());
        let size = window
            .contentView()
            .map(|view| view.bounds().size)
            .unwrap_or(CGSize::new(CONTENT_WIDTH, CONTENT_HEIGHT));
        let width = if size.width.is_finite() {
            size.width.max(0.0)
        } else {
            0.0
        };
        let height = if size.height.is_finite() {
            size.height.max(0.0)
        } else {
            0.0
        };
        let layer = &self.ivars().layer;
        layer.setContentsScale(scale);
        layer.setFrame(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width, height),
        ));
        layer.setDrawableSize(CGSize::new(width * scale, height * scale));
        self.ivars().link.post(Command::Metrics(metrics(
            width as f32,
            height as f32,
            scale as f32,
        )));
    }

    fn post_settings(&self) {
        self.ivars()
            .link
            .post(Command::Settings(current_settings()));
    }
}

fn current_settings() -> session::PlatformSettings {
    let locale = NSLocale::currentLocale()
        .localeIdentifier()
        .to_string()
        .replace('_', "-");
    let mut text_scale = 1.0;
    let mut brightness = session::Brightness::Light;
    let mut reduce_motion = false;
    let mut high_contrast = false;
    let mut bold_text = false;
    if let Some(mtm) = MainThreadMarker::new() {
        let name = NSApplication::sharedApplication(mtm)
            .effectiveAppearance()
            .name()
            .to_string();
        brightness = appearance_brightness(&name);
        let workspace = NSWorkspace::sharedWorkspace();
        reduce_motion = workspace.accessibilityDisplayShouldReduceMotion();
        high_contrast = workspace.accessibilityDisplayShouldIncreaseContrast();
        bold_text = unsafe { msg_send![&*workspace, accessibilityDisplayShouldPreferBoldText] };
        let options: Retained<NSDictionary<objc2_app_kit::NSFontTextStyleOptionKey, AnyObject>> =
            NSDictionary::new();
        let font =
            unsafe { NSFont::preferredFontForTextStyle_options(NSFontTextStyleBody, &options) };
        text_scale = text_scale_from_point_size(font.pointSize());
    }
    session::PlatformSettings {
        locale,
        text_scale,
        brightness,
        reduce_motion,
        bold_text,
        high_contrast,
    }
}

/// `Dark` when the appearance name contains `Dark`.
pub(crate) fn appearance_brightness(name: &str) -> session::Brightness {
    if name.contains("Dark") {
        session::Brightness::Dark
    } else {
        session::Brightness::Light
    }
}

/// Body point size divided by 13, clamped to `[0.5, 3]`. Non-finite or non-positive sizes are 1.
pub(crate) fn text_scale_from_point_size(point_size: f64) -> f32 {
    if !point_size.is_finite() || point_size <= 0.0 {
        return 1.0;
    }
    let scale = (point_size / 13.0) as f32;
    if !scale.is_finite() {
        return 1.0;
    }
    scale.clamp(0.5, 3.0)
}

fn command_key(selector: &str) -> Option<session::KeyPress> {
    let (key, shift) = match selector {
        "insertNewline:" => (Key::Enter, false),
        "insertTab:" => (Key::Tab, false),
        "insertBacktab:" => (Key::Tab, true),
        "cancelOperation:" => (Key::Escape, false),
        "deleteForward:" => (Key::Delete, false),
        "deleteWordBackward:" => (Key::DeleteWord, false),
        "deleteWordForward:" => (Key::DeleteWordForward, false),
        "deleteToBeginningOfLine:" => (Key::DeleteToLineStart, false),
        "deleteToEndOfLine:" => (Key::DeleteToLineEnd, false),
        "moveLeft:" => (Key::Left, false),
        "moveRight:" => (Key::Right, false),
        "moveUp:" => (Key::Up, false),
        "moveDown:" => (Key::Down, false),
        "moveLeftAndModifySelection:" => (Key::Left, true),
        "moveRightAndModifySelection:" => (Key::Right, true),
        "moveUpAndModifySelection:" => (Key::Up, true),
        "moveDownAndModifySelection:" => (Key::Down, true),
        "moveWordLeft:" => (Key::WordLeft, false),
        "moveWordRight:" => (Key::WordRight, false),
        "moveWordLeftAndModifySelection:" => (Key::WordLeft, true),
        "moveWordRightAndModifySelection:" => (Key::WordRight, true),
        "moveToBeginningOfLine:" | "moveToBeginningOfParagraph:" => (Key::Home, false),
        "moveToEndOfLine:" | "moveToEndOfParagraph:" => (Key::End, false),
        "moveToBeginningOfLineAndModifySelection:"
        | "moveToBeginningOfParagraphAndModifySelection:" => (Key::Home, true),
        "moveToEndOfLineAndModifySelection:" | "moveToEndOfParagraphAndModifySelection:" => {
            (Key::End, true)
        }
        "moveToBeginningOfDocument:" => (Key::DocumentStart, false),
        "moveToEndOfDocument:" => (Key::DocumentEnd, false),
        "moveToBeginningOfDocumentAndModifySelection:" => (Key::DocumentStart, true),
        "moveToEndOfDocumentAndModifySelection:" => (Key::DocumentEnd, true),
        _ => return None,
    };
    Some(session::KeyPress {
        key,
        shift,
        modifiers: Modifiers::empty(),
    })
}

struct MacServices {
    font: Vec<u8>,
    ime_view: Arc<AtomicPtr<c_void>>,
}

impl MacServices {
    /// Reads `IMPASTO_LOG` once. The default sink keeps `info`.
    fn filter() -> LogLevel {
        static FILTER: std::sync::OnceLock<LogLevel> = std::sync::OnceLock::new();
        *FILTER.get_or_init(|| {
            std::env::var("IMPASTO_LOG")
                .map(|text| LogLevel::parse(&text))
                .unwrap_or(LogLevel::Info)
        })
    }
}

impl Services for MacServices {
    fn font(&self, family: &str) -> Option<&[u8]> {
        (family == FONT_FAMILY).then_some(self.font.as_slice())
    }

    fn image<'a>(&'a self, _source: &str) -> Option<engine::ImagePixels<'a>> {
        None
    }

    fn semantics(&self, update: &uicore::SemanticsUpdate) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        let update = update.clone();
        dispatch_main(Box::new(move || {
            // SAFETY: the address is the content view, stored on the main thread
            // before the run loop, and this closure also runs on the main thread.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_semantics(update);
        }));
    }

    fn extra_fonts(&self) -> Vec<(String, Vec<u8>)> {
        const PATHS: &[&str] = &[
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
        ];
        for path in PATHS {
            if let Ok(bytes) = std::fs::read(path)
                && !bytes.is_empty()
            {
                return vec![("SansCJK".to_string(), bytes)];
            }
        }
        Vec::new()
    }

    fn log(&self, level: LogLevel, message: &str) {
        if level > Self::filter() {
            return;
        }
        let prefix = match level {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        };
        match level {
            LogLevel::Error | LogLevel::Warn => eprintln!("impasto {prefix}: {message}"),
            LogLevel::Info | LogLevel::Debug => println!("impasto {prefix}: {message}"),
        }
    }

    fn apply_ime(&self, update: &session::ImeUpdate) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        let update = update.clone();
        dispatch_main(Box::new(move || {
            // SAFETY: the address is the content view, stored on the main thread before
            // the run loop, and this closure also runs on the main thread. The window
            // retains the view for the process lifetime.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_ime_update(&update);
        }));
    }
}

/// A script session. The shell paints the layer tree that session records.
pub enum Launch {
    Script { name: String, source: String },
}

pub enum RunError {
    Boot(BootError),
    Window(anyhow::Error),
    Dump,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Boot(error) => write!(formatter, "{error}"),
            Self::Window(error) => write!(formatter, "{error:#}"),
            Self::Dump => write!(formatter, "frame dump failed"),
        }
    }
}

pub fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Boot(error) => error.code(),
        RunError::Window(_) | RunError::Dump => 1,
    }
}

pub fn run(
    launch: Launch,
    assets: PathBuf,
    font: &[u8],
    dump_list: bool,
    initial_url: Option<String>,
) -> Result<(), RunError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(RunError::Window(anyhow::anyhow!(
            "AppKit must start on the main thread"
        )));
    };
    let vsync = DisplayLink::new().map_err(RunError::Window)?;
    let dump = std::env::var_os("IMPASTO_DUMP").map(PathBuf::from);
    let exit_code = Arc::new(AtomicI32::new(0));
    let exit_for_stop = Arc::clone(&exit_code);
    let on_stop: Arc<dyn Fn(i32) + Send + Sync> = Arc::new(move |code| {
        exit_for_stop.store(code, Ordering::SeqCst);
        wake_main();
    });
    let Launch::Script { name, source } = launch;
    let shell_launch = ShellLaunch::Session(Box::new(move |slot, io, host| {
        quickjs::open_with_io(
            &source,
            &name,
            slot.shaper(),
            slot.log_hook(),
            Some(io),
            host,
        )
        .map_err(|error| match error {
            quickjs::Error::NotLinked => SessionError::NotLinked,
            quickjs::Error::Runtime => SessionError::Runtime,
            quickjs::Error::Script(text) => SessionError::Script(text),
        })
    }));
    let dump_for_surface = dump.clone();
    let on_stop_for_surface = Arc::clone(&on_stop);
    let ime_view = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
    let decoder = Arc::new(PngDecoder { directory: assets });
    let launch_url = session::shared(initial_url.clone());
    let mac = Arc::new(MacBridge);
    let link = hostshell::start(
        Boot {
            font_family: FONT_FAMILY.to_string(),
            launch: shell_launch,
            services: Box::new(MacServices {
                font: font.to_vec(),
                ime_view: Arc::clone(&ime_view),
            }),
            vsync: Arc::new(vsync),
            decoder,
            modules: Vec::new(),
            main_thread: Some(Arc::new(|work| dispatch_main(work))),
            launch_url: Arc::clone(&launch_url),
            http: Some(
                hostshell::http_client()
                    .map_err(|message| RunError::Window(anyhow::anyhow!(message)))?,
            ),
            open_surface: Box::new(move |_engine, target| {
                Ok(Box::new(MetalSurface::new(
                    target.layer,
                    dump_for_surface.clone(),
                    Arc::clone(&on_stop_for_surface),
                )))
            }),
            dump_list,
        }
        .with_module(hostshell::clipboard(Arc::clone(&mac) as _))
        .with_module(hostshell::linking(
            Arc::clone(&mac) as _,
            Arc::clone(&launch_url),
        ))
        .with_module(hostshell::file_system(Arc::clone(&mac) as _))
        .with_module(hostshell::async_storage(Arc::clone(&mac) as _))
        .with_module(hostshell::permissions(mac as _)),
    )
    .map_err(RunError::Boot)?;

    let app = NSApplication::sharedApplication(mtm);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let frame = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(CONTENT_WIDTH, CONTENT_HEIGHT),
    );
    let window = unsafe {
        // SAFETY: the style is titled, closable, miniaturizable, and resizable. Buffered
        // backing and `defer = false` create the window before this returns.
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&NSString::from_str("impasto counter"));
    window.setAcceptsMouseMovedEvents(true);
    // SAFETY: `HostState` keeps this window retained until the process leaves `run`.
    unsafe { window.setReleasedWhenClosed(false) };

    let scale = point_scale(&window, mtm);
    let Some(device) = MTLCreateSystemDefaultDevice() else {
        return Err(RunError::Window(anyhow::anyhow!(
            "Metal device was not created"
        )));
    };
    let layer = CAMetalLayer::layer();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(false);
    layer.setContentsScale(scale);
    layer.setFrame(frame);
    layer.setDrawableSize(CGSize::new(CONTENT_WIDTH * scale, CONTENT_HEIGHT * scale));

    let view = ContentView::new(mtm, frame, link.poster());
    ime_view.store(Retained::as_ptr(&view).cast_mut().cast(), Ordering::Release);
    view.setWantsLayer(true);
    let backing: &CALayer = layer.as_ref();
    view.setLayer(Some(backing));
    window.setContentView(Some(&*view));

    let host = Host::new(
        mtm,
        HostState {
            window: window.clone(),
            layer,
            link,
            initial_url,
            launched: Cell::new(false),
        },
    );
    host.ivars()
        .window
        .setDelegate(Some(ProtocolObject::from_ref(&*host)));
    app.setDelegate(Some(ProtocolObject::from_ref(&*host)));
    watch_appearance(&host);
    app.run();
    if exit_code.load(Ordering::SeqCst) != 0 {
        return Err(RunError::Dump);
    }
    Ok(())
}

fn watch_appearance(host: &Host) {
    let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .addObserver_selector_name_object_suspensionBehavior(
                host,
                objc2::sel!(themeChanged:),
                Some(&name),
                None,
                NSNotificationSuspensionBehavior::DeliverImmediately,
            );
        NSWorkspace::sharedWorkspace()
            .notificationCenter()
            .addObserver_selector_name_object(
                host,
                objc2::sel!(themeChanged:),
                Some(objc2_app_kit::NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
            );
    }
}

struct PngDecoder {
    directory: PathBuf,
}

impl hostshell::ImageDecoder for PngDecoder {
    fn decode(&self, source: &str) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        self.decode_at(source, 1)
    }

    fn decode_at(
        &self,
        source: &str,
        scale_bucket: u8,
    ) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        let scale = f32::from(scale_bucket.max(1));
        let Some(path) = crate::assets::resolve_asset_path(&self.directory, source, scale) else {
            return Err(hostshell::ImageError::Missing);
        };
        let rgba = crate::pixels::load_rgba_png(&path)
            .map_err(|error| hostshell::ImageError::Failed(error.to_string()))?;
        Ok(image_handle(source, rgba))
    }

    fn decode_bytes(
        &self,
        source: &str,
        bytes: &[u8],
    ) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        let rgba = crate::pixels::load_rgba_bytes(bytes)
            .map_err(|error| hostshell::ImageError::Failed(error.to_string()))?;
        Ok(image_handle(source, rgba))
    }
}

fn image_handle(source: &str, rgba: crate::pixels::Rgba) -> hostshell::ImageHandle {
    hostshell::ImageHandle {
        source: Arc::from(source),
        generation: 0,
        size: [rgba.width, rgba.height],
        pixels: Arc::from(rgba.pixels),
    }
}

fn point_scale(window: &NSWindow, mtm: MainThreadMarker) -> f64 {
    let screen = window.screen().or_else(|| NSScreen::mainScreen(mtm));
    let factor = screen
        .map(|screen| screen.backingScaleFactor())
        .unwrap_or(0.0);
    if factor > 0.0 { factor } else { 2.0 }
}

#[cfg(test)]
mod settings_tests {
    use session::{Brightness, Key};

    use super::{appearance_brightness, command_key, text_scale_from_point_size};

    #[test]
    fn dark_appearance_names_are_dark() {
        assert_eq!(
            appearance_brightness("NSAppearanceNameDarkAqua"),
            Brightness::Dark
        );
        assert_eq!(
            appearance_brightness("NSAppearanceNameAqua"),
            Brightness::Light
        );
        assert_eq!(
            appearance_brightness("NSAppearanceNameAccessibilityHighContrastDarkAqua"),
            Brightness::Dark
        );
    }

    #[test]
    fn text_scale_uses_body_point_size_over_thirteen() {
        assert_eq!(text_scale_from_point_size(13.0), 1.0);
        assert_eq!(text_scale_from_point_size(26.0), 2.0);
        assert_eq!(text_scale_from_point_size(0.0), 1.0);
        assert_eq!(text_scale_from_point_size(f64::NAN), 1.0);
        assert_eq!(text_scale_from_point_size(100.0), 3.0);
    }

    #[test]
    fn backtab_sets_shift_and_tab_does_not() {
        let Some(tab) = command_key("insertTab:") else {
            panic!("tab");
        };
        assert_eq!(tab.key, Key::Tab);
        assert!(!tab.shift);
        let Some(back) = command_key("insertBacktab:") else {
            panic!("backtab");
        };
        assert_eq!(back.key, Key::Tab);
        assert!(back.shift);
        assert!(command_key("copy:").is_none());
    }
}

#[cfg(test)]
mod url_tests {
    use super::{nonempty_url, skip_launch_duplicate};

    #[test]
    fn empty_text_is_not_a_url() {
        assert_eq!(nonempty_url(""), None);
        assert_eq!(nonempty_url("  "), None);
        assert_eq!(
            nonempty_url("https://app.example/item"),
            Some("https://app.example/item".into())
        );
        assert_eq!(nonempty_url(" myapp://open "), Some("myapp://open".into()));
    }

    #[test]
    fn launch_duplicate_is_skipped_only_before_did_finish_launching() {
        let initial = Some("https://app.example/item");
        assert!(skip_launch_duplicate(
            false,
            initial,
            "https://app.example/item"
        ));
        assert!(!skip_launch_duplicate(
            false,
            initial,
            "https://app.example/other"
        ));
        assert!(!skip_launch_duplicate(
            true,
            initial,
            "https://app.example/item"
        ));
        assert!(!skip_launch_duplicate(
            false,
            None,
            "https://app.example/item"
        ));
    }
}

struct MacBridge;

impl ClipboardHost for MacBridge {
    fn read_clipboard(&self) -> String {
        read_pasteboard()
    }

    fn write_clipboard(&self, text: &str) {
        write_pasteboard(text);
    }
}

impl LinkingHost for MacBridge {
    fn open_url(&self, url: &str) -> Result<(), String> {
        open_url_main(url)
    }
}

impl StorageHost for MacBridge {
    fn document_dir(&self) -> Result<String, String> {
        document_dir_main()
    }

    fn pick_document(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
}

impl PermissionsHost for MacBridge {
    fn permission_status(&self, name: &str) -> String {
        permission_status_main(name)
    }

    fn request_permissions(&self, names: &[String]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|name| (name.clone(), permission_status_main(name)))
            .collect()
    }

    fn permission_rationale(&self, _name: &str) -> bool {
        false
    }

    fn service_status(&self, _name: &str) -> String {
        "notApplicable".to_owned()
    }

    fn open_app_settings(&self) -> bool {
        open_settings_main().is_ok()
    }
}

fn document_dir_main() -> Result<String, String> {
    let Some(manager) = objc2::runtime::AnyClass::get(c"NSFileManager") else {
        return Err("module \"FileSystem\" is not linked".to_owned());
    };
    let shared: *mut AnyObject = unsafe { msg_send![manager, defaultManager] };
    if shared.is_null() {
        return Err("module \"FileSystem\" is not linked".to_owned());
    }
    let url: *mut AnyObject = unsafe {
        msg_send![shared, URLForDirectory: 9usize, inDomain: 1usize, appropriateForURL: std::ptr::null::<AnyObject>(), create: true, error: std::ptr::null_mut::<*mut AnyObject>()]
    };
    if url.is_null() {
        return Err("module \"FileSystem\" is not linked".to_owned());
    }
    let path: *mut AnyObject = unsafe { msg_send![url, path] };
    if path.is_null() {
        return Err("module \"FileSystem\" is not linked".to_owned());
    }
    let text = unsafe { &*path.cast::<NSString>() };
    Ok(text.to_string())
}

fn permission_status_main(name: &str) -> String {
    match name {
        "camera" | "microphone" => media_status(name),
        _ => "denied".to_owned(),
    }
}

fn media_status(name: &str) -> String {
    let Some(device) = objc2::runtime::AnyClass::get(c"AVCaptureDevice") else {
        return "denied".to_owned();
    };
    let media = NSString::from_str(if name == "camera" { "vide" } else { "soun" });
    let status: i64 = unsafe { msg_send![device, authorizationStatusForMediaType: &*media] };
    match status {
        3 => "granted".to_owned(),
        1 => "restricted".to_owned(),
        2 => "denied".to_owned(),
        _ => "denied".to_owned(),
    }
}

fn open_settings_main() -> Result<(), String> {
    open_url_main("x-apple.systempreferences:com.apple.preference.security?Privacy")
}

fn open_url_main(url: &str) -> Result<(), String> {
    let Some(workspace) = objc2::runtime::AnyClass::get(c"NSWorkspace") else {
        return Err("openURL is not linked".to_owned());
    };
    let Some(urls) = objc2::runtime::AnyClass::get(c"NSURL") else {
        return Err("openURL is not linked".to_owned());
    };
    let shared: *mut AnyObject = unsafe { msg_send![workspace, sharedWorkspace] };
    if shared.is_null() {
        return Err("openURL is not linked".to_owned());
    }
    let text = NSString::from_str(url);
    let nsurl: *mut AnyObject = unsafe { msg_send![urls, URLWithString: &*text] };
    if nsurl.is_null() {
        return Err("unsupported URL".to_owned());
    }
    let opened: bool = unsafe { msg_send![shared, openURL: nsurl] };
    if opened {
        Ok(())
    } else {
        Err("openURL failed".to_owned())
    }
}

fn write_pasteboard(text: &str) {
    let Some(class) = objc2::runtime::AnyClass::get(c"NSPasteboard") else {
        return;
    };
    let pasteboard: *mut AnyObject = unsafe { msg_send![class, generalPasteboard] };
    if pasteboard.is_null() {
        return;
    }
    let value = NSString::from_str(text);
    let kind = NSString::from_str("public.utf8-plain-text");
    let _: () = unsafe { msg_send![pasteboard, clearContents] };
    let _: bool = unsafe { msg_send![pasteboard, setString: &*value, forType: &*kind] };
}

fn read_pasteboard_html() -> Option<String> {
    let text = pasteboard_string("public.html");
    if text.is_empty() { None } else { Some(text) }
}

fn read_pasteboard() -> String {
    pasteboard_string("public.utf8-plain-text")
}

fn pasteboard_string(kind: &str) -> String {
    let Some(class) = objc2::runtime::AnyClass::get(c"NSPasteboard") else {
        return String::new();
    };
    let pasteboard: *mut AnyObject = unsafe { msg_send![class, generalPasteboard] };
    if pasteboard.is_null() {
        return String::new();
    }
    let kind = NSString::from_str(kind);
    let value: *mut AnyObject = unsafe { msg_send![pasteboard, stringForType: &*kind] };
    if value.is_null() {
        return String::new();
    }
    unsafe { &*value.cast::<NSString>() }.to_string()
}

fn wake_main() {
    dispatch_main(Box::new(|| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        NSApplication::sharedApplication(mtm).terminate(None);
    }));
}

fn dispatch_main(work: Box<dyn FnOnce() + Send>) {
    let context = Box::into_raw(Box::new(work));
    // SAFETY: `_dispatch_main_q` is the queue behind `dispatch_get_main_queue()`.
    // `context` is a boxed closure. The callback takes ownership of it.
    unsafe {
        let queue = std::ptr::addr_of!(_dispatch_main_q).cast_mut().cast();
        dispatch_async_f(queue, context.cast(), run_on_main);
    }
}

unsafe extern "C" fn run_on_main(context: *mut c_void) {
    let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
    work();
}

unsafe extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}
