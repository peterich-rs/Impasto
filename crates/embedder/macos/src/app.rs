//! AppKit window. The render thread owns the engine and the session.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

use embedder::{
    Boot, BootError, Command, Launch as EmbedderLaunch, Link, NativePtr, Poster, Services,
    SessionError, SurfaceTarget, metrics,
};
use engine::session::{InputEvent, Key};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, available, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSCursor, NSEvent, NSEventModifierFlags, NSEventPhase, NSFont, NSFontTextStyleBody, NSScreen,
    NSTextInputClient, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
    NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSArray, NSAttributedString, NSDictionary, NSDistributedNotificationCenter, NSLocale,
    NSNotFound, NSNotification, NSNotificationSuspensionBehavior, NSPoint, NSRange, NSRect,
    NSString, NSUInteger, NSURL,
};

use crate::text_client::Marked;
use objc2_quartz_core::{CALayer, CAMetalLayer};

use crate::menu;
use crate::pointer::{PointerSample, pointer_event, scene_f32};
use crate::surface::MetalSurface;
use crate::vsync::DisplayLink;
use engine::input::{Button, Modifiers, Phase, PointerEvent, ScrollPhase};

use crate::window::WindowOptions;
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

fn scroll_phase_of(phase: NSEventPhase, momentum: NSEventPhase) -> ScrollPhase {
    if !phase.is_empty() {
        if phase.contains(NSEventPhase::Cancelled) {
            return ScrollPhase::Cancelled;
        }
        if phase.contains(NSEventPhase::Ended) {
            return ScrollPhase::Ended;
        }
        if phase.contains(NSEventPhase::Began) {
            return ScrollPhase::Began;
        }
        if phase.contains(NSEventPhase::MayBegin) {
            return ScrollPhase::MayBegin;
        }
        return ScrollPhase::Changed;
    }
    if momentum.intersects(NSEventPhase::Ended | NSEventPhase::Cancelled) {
        return ScrollPhase::MomentumEnded;
    }
    if momentum.contains(NSEventPhase::Began) {
        return ScrollPhase::MomentumBegan;
    }
    if momentum.is_empty() {
        ScrollPhase::Discrete
    } else {
        ScrollPhase::MomentumChanged
    }
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

    // SAFETY: ContentView is an NSView subclass created only on the main thread.
    unsafe impl NSObjectProtocol for ContentView {}

    #[allow(
        non_snake_case,
        reason = "NSTextInputClient method names are the AppKit selectors"
    )]
    // SAFETY: The methods below are the NSTextInputClient selectors AppKit calls on the main thread.
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
                // SAFETY: AppKit passes a writable range; the null pointer returned above.
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

        #[unsafe(method(copy:))]
        fn copy_from_menu(&self, _sender: Option<&AnyObject>) {
            self.post_edits(vec![InputEvent::Copy]);
        }

        #[unsafe(method(cut:))]
        fn cut_from_menu(&self, _sender: Option<&AnyObject>) {
            self.post_edits(vec![InputEvent::Cut]);
        }

        #[unsafe(method(paste:))]
        fn paste_from_menu(&self, _sender: Option<&AnyObject>) {
            self.post_edits(vec![InputEvent::Paste {
                plain: read_pasteboard(), html: read_pasteboard_html(),
            }]);
        }

        #[unsafe(method(selectAll:))]
        fn select_all_from_menu(&self, _sender: Option<&AnyObject>) {
            self.post_edits(vec![InputEvent::Key(engine::session::KeyPress {
                key: Key::Char('a'), modifiers: Modifiers::empty().with(Modifiers::META), shift: false,
            })]);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let modifiers = modifiers_of(event);
            let chord = modifiers.contains(Modifiers::CONTROL)
                || modifiers.contains(Modifiers::ALT)
                || modifiers.contains(Modifiers::META);
            if chord && let Some(ch) = shortcut_character(event) {
                self.post_edits(vec![InputEvent::Key(engine::session::KeyPress {
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
                scene_f32(event.magnification()),
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
            if NSEvent::pressedMouseButtons() == 0 {
                NSCursor::arrowCursor().set();
            }
            self.post_outside(event);
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            // SAFETY: this is NSView's updateTrackingAreas, called by AppKit on the main thread.
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
        // SAFETY: initWithFrame: is NSView's designated initializer and the ivars are already installed.
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
        // SAFETY: the rect is this view's bounds, and `self` is the tracking owner on the main thread.
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
        // AppKit keeps sending real window-relative positions during a drag.
        if NSEvent::pressedMouseButtons() != 0 {
            return;
        }
        self.ivars().poster.post(Command::Pointer(PointerEvent {
            pointer: engine::input::PointerId(0),
            kind: engine::input::PointerKind::Mouse,
            phase: Phase::Move,
            x: -1.0,
            y: -1.0,
            time_ms: event.timestamp() * 1000.0,
            button: Button::None,
            buttons: engine::input::Buttons::empty(),
            modifiers: modifiers_of(event),
            dx: 0.0,
            dy: 0.0,
            scroll_phase: ScrollPhase::Discrete,
            zoom: 0.0,
            pen: None,
        }));
    }

    fn enqueue(&self, phase: Phase, event: &NSEvent, button: Button) {
        let local = self.convertPoint_fromView(event.locationInWindow(), None);
        let height = self.bounds().size.height;
        let modifiers = modifiers_of(event);
        let (dx, dy, scroll_phase) = if phase == Phase::Wheel {
            // AppKit reports precise deltas in points and coarse deltas in lines.
            const WHEEL_LINE_POINTS: f64 = 40.0;
            let scale = if event.hasPreciseScrollingDeltas() {
                1.0
            } else {
                WHEEL_LINE_POINTS
            };
            (
                scene_f32(-event.scrollingDeltaX() * scale),
                scene_f32(-event.scrollingDeltaY() * scale),
                scroll_phase_of(event.phase(), event.momentumPhase()),
            )
        } else {
            (0.0, 0.0, ScrollPhase::Discrete)
        };
        self.ivars().poster.post(Command::Pointer(pointer_event(
            PointerSample {
                phase,
                local_x: local.x,
                local_y: local.y,
                view_height: height,
                time_ms: event.timestamp() * 1000.0,
                scroll_phase,
            },
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
                tracing::debug!(target: "impasto::shell", %name, "text command ignored");
                return;
            }
        };
        self.post_edits(events);
    }

    fn scene_rect_to_screen(&self, rect: engine::Rect) -> NSRect {
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

    fn apply_semantics(&self, nodes: &[embedder::ProjectedNode]) {
        self.ivars().a11y.apply(
            self.mtm(),
            &self.ivars().poster,
            self as *const ContentView as *const NSView,
            nodes,
        );
    }

    fn apply_ime_update(&self, update: &engine::session::ImeUpdate) {
        let Some(window) = self.window() else {
            return;
        };
        match update {
            engine::session::ImeUpdate::Hide => {
                self.ivars().wants_ime.set(false);
                let _: bool = window.makeFirstResponder(None);
            }
            engine::session::ImeUpdate::Show(state) | engine::session::ImeUpdate::Update(state) => {
                self.ivars().marked.borrow_mut().apply(state);
                self.ivars().wants_ime.set(true);
                if matches!(update, engine::session::ImeUpdate::Show(_)) {
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
    surface: Cell<Option<embedder::SurfaceId>>,
    link: Link,
    /// `--url` at process start. `set_launch_url` does not overwrite this copy.
    initial_url: Option<String>,
    native: RefCell<Option<crate::native::Host>>,
    detached: Cell<bool>,
    launched: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = HostState]
    struct Host;

    // SAFETY: Host is an NSObject created only on the main thread.
    unsafe impl NSObjectProtocol for Host {}

    // SAFETY: These selectors are the NSApplicationDelegate methods AppKit calls on the main thread.
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
                let handler = self.ivars().native.borrow().as_ref().map(crate::native::Host::url_handler);
                if handler.is_some_and(|open| open(&url)) {
                    continue;
                }
                self.ivars().link.post(Command::LaunchUrl(url));
            }
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            self.detach();
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn did_become_active(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(engine::session::Lifecycle::Resumed));
            self.post_settings();
        }

        #[unsafe(method(applicationWillResignActive:))]
        fn will_resign_active(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(engine::session::Lifecycle::Inactive));
        }

        #[unsafe(method(applicationDidHide:))]
        fn did_hide(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(engine::session::Lifecycle::Hidden));
        }


    }

    impl Host {
        #[unsafe(method(themeChanged:))]
        fn theme_changed(&self, _notification: &NSNotification) {
            self.post_settings();
        }

        #[unsafe(method(applicationDidReceiveMemoryWarning:))]
        fn did_receive_memory_warning(&self, _notification: &NSNotification) {
            self.ivars().link.post(Command::MemoryWarning);
        }
    }

    // SAFETY: These selectors are the NSWindowDelegate methods AppKit calls on the main thread.
    unsafe impl NSWindowDelegate for Host {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            self.detach();
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
                .post(Command::Lifecycle(engine::session::Lifecycle::Hidden));
        }

        #[unsafe(method(windowDidDeminiaturize:))]
        fn window_did_deminiaturize(&self, _notification: &NSNotification) {
            self.ivars()
                .link
                .post(Command::Lifecycle(engine::session::Lifecycle::Resumed));
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
    fn detach(&self) {
        let state = self.ivars();
        if !state.detached.replace(true) {
            let native = state.native.borrow_mut().take();
            drop(native);
            state
                .link
                .post(Command::Lifecycle(engine::session::Lifecycle::Detached));
            let _ = state.link.shutdown_and_join();
        }
    }

    fn new(mtm: MainThreadMarker, state: HostState) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: `init` is NSObject's designated initializer, and the ivars are already installed.
        unsafe { msg_send![super(this), init] }
    }

    fn finish_launching(&self) {
        let window = &self.ivars().window;
        window.makeKeyAndOrderFront(None);
        let app = NSApplication::sharedApplication(self.mtm());
        if available!(macos = 14.0) {
            app.activate();
        } else {
            #[allow(
                deprecated,
                reason = "activateIgnoringOtherApps is the pre-macOS 14 way to become active"
            )]
            app.activateIgnoringOtherApps(true);
        }
        let scale = point_scale(window, self.mtm());
        let layer = Retained::as_ptr(&self.ivars().layer) as *mut c_void;
        let size = self.update_drawable(scale);
        let metrics = metrics(
            scene_f32(size.width),
            scene_f32(size.height),
            scene_f32(scale),
        );
        let surface_id = embedder::SurfaceId::fresh();
        self.ivars().surface.set(Some(surface_id));
        self.ivars()
            .link
            .post(Command::SurfaceReady(SurfaceTarget::new(
                surface_id,
                NativePtr(layer),
                metrics,
            )));
        self.post_settings();
        tracing::info!(target: "impasto::shell", "host ready");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }

    fn update_drawable(&self, scale: f64) -> CGSize {
        let window = &self.ivars().window;
        let mut size = window
            .contentView()
            .map_or(window.contentMinSize(), |view| view.bounds().size);
        size.width = if size.width.is_finite() {
            size.width.max(0.0)
        } else {
            0.0
        };
        size.height = if size.height.is_finite() {
            size.height.max(0.0)
        } else {
            0.0
        };
        let layer = &self.ivars().layer;
        layer.setContentsScale(scale);
        layer.setFrame(CGRect::new(CGPoint::new(0.0, 0.0), size));
        layer.setDrawableSize(CGSize::new(size.width * scale, size.height * scale));
        size
    }

    fn post_metrics(&self) {
        let scale = point_scale(&self.ivars().window, self.mtm());
        let size = self.update_drawable(scale);
        let command = resize_command(self.ivars().surface.get(), size.width, size.height, scale);
        self.ivars().link.post(command);
    }

    fn post_settings(&self) {
        self.ivars()
            .link
            .post(Command::Settings(current_settings()));
    }
}

fn resize_command(
    surface: Option<embedder::SurfaceId>,
    width: f64,
    height: f64,
    scale: f64,
) -> Command {
    let metrics = metrics(scene_f32(width), scene_f32(height), scene_f32(scale));
    if let Some(id) = surface {
        Command::SurfaceChanged {
            id,
            metrics,
            pixels: embedder::pixel_size(metrics),
        }
    } else {
        Command::Metrics(metrics)
    }
}

fn current_settings() -> engine::session::PlatformSettings {
    let locale = NSLocale::currentLocale()
        .localeIdentifier()
        .to_string()
        .replace('_', "-");
    let mut text_scale = 1.0;
    let mut brightness = engine::session::Brightness::Light;
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
        let bold_selector = objc2::sel!(accessibilityDisplayShouldPreferBoldText);
        if workspace.respondsToSelector(bold_selector) {
            // SAFETY: the runtime confirmed this optional selector is implemented.
            bold_text = unsafe { msg_send![&*workspace, accessibilityDisplayShouldPreferBoldText] };
        }
        let options: Retained<NSDictionary<objc2_app_kit::NSFontTextStyleOptionKey, AnyObject>> =
            NSDictionary::new();
        let font = {
            // SAFETY: `options` is empty, the documented default for the body font.
            unsafe { NSFont::preferredFontForTextStyle_options(NSFontTextStyleBody, &options) }
        };
        text_scale = text_scale_from_point_size(font.pointSize());
    }
    engine::session::PlatformSettings {
        locale,
        text_scale,
        brightness,
        reduce_motion,
        bold_text,
        high_contrast,
    }
}

/// `Dark` when the appearance name contains `Dark`.
pub(crate) fn appearance_brightness(name: &str) -> engine::session::Brightness {
    if name.contains("Dark") {
        engine::session::Brightness::Dark
    } else {
        engine::session::Brightness::Light
    }
}

/// Body point size divided by 13, clamped to `[0.5, 3]`. Non-finite or non-positive sizes are 1.
pub(crate) fn text_scale_from_point_size(point_size: f64) -> f32 {
    if !point_size.is_finite() || point_size <= 0.0 {
        return 1.0;
    }
    let scale = scene_f32(point_size / 13.0);
    if !scale.is_finite() {
        return 1.0;
    }
    scale.clamp(0.5, 3.0)
}

fn command_key(selector: &str) -> Option<engine::session::KeyPress> {
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
    Some(engine::session::KeyPress {
        key,
        shift,
        modifiers: Modifiers::empty(),
    })
}

struct MacServices {
    font: Vec<u8>,
    ime_view: Arc<AtomicPtr<c_void>>,
}

/// Source maps larger than this stay off the panel. The bundle's map is small.
const SOURCE_MAP_CAP: u64 = 512 * 1024;

fn load_source_map(script: &str) -> Option<Arc<String>> {
    let path = Path::new(script).with_extension("js.map");
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > SOURCE_MAP_CAP {
        return None;
    }
    std::fs::read_to_string(path).ok().map(Arc::new)
}

impl Services for MacServices {
    fn font(&self, family: &str) -> Option<&[u8]> {
        (family == FONT_FAMILY).then_some(self.font.as_slice())
    }

    fn semantics(&self, nodes: &[embedder::ProjectedNode]) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        let nodes = nodes.to_vec();
        on_main(Box::new(move || {
            // SAFETY: the address is the content view stored on the main thread, and this closure runs there too.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_semantics(&nodes);
        }));
    }

    fn set_cursor(&self, cursor: engine::style::CursorStyle) {
        use engine::style::CursorStyle;

        on_main(Box::new(move || {
            let cursor = match cursor {
                CursorStyle::Auto | CursorStyle::Default => NSCursor::arrowCursor(),
                CursorStyle::Pointer => NSCursor::pointingHandCursor(),
                CursorStyle::Text => NSCursor::IBeamCursor(),
                CursorStyle::Grab | CursorStyle::Move => NSCursor::openHandCursor(),
                CursorStyle::Grabbing => NSCursor::closedHandCursor(),
                CursorStyle::NotAllowed => NSCursor::operationNotAllowedCursor(),
                CursorStyle::Crosshair => NSCursor::crosshairCursor(),
                CursorStyle::ColResize => {
                    if available!(macos = 15.0) {
                        NSCursor::columnResizeCursor()
                    } else {
                        #[allow(deprecated, reason = "system column cursor on macOS 13 and 14")]
                        NSCursor::resizeLeftRightCursor()
                    }
                }
                CursorStyle::RowResize => {
                    if available!(macos = 15.0) {
                        NSCursor::rowResizeCursor()
                    } else {
                        #[allow(deprecated, reason = "system row cursor on macOS 13 and 14")]
                        NSCursor::resizeUpDownCursor()
                    }
                }
            };
            cursor.set();
        }));
    }

    fn extra_fonts(&self) -> Vec<(String, Vec<u8>)> {
        let candidates: &[(&str, &[&str])] = &[
            (
                "SansCJK",
                &[
                    "/System/Library/Fonts/Hiragino Sans GB.ttc",
                    "/System/Library/Fonts/STHeiti Medium.ttc",
                    "/System/Library/Fonts/Supplemental/Songti.ttc",
                ],
            ),
            (
                "Monospace",
                &[
                    "/System/Library/Fonts/SFNSMono.ttf",
                    "/System/Library/Fonts/Menlo.ttc",
                    "/System/Library/Fonts/Monaco.ttf",
                    "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
                ],
            ),
        ];
        candidates
            .iter()
            .filter_map(|(family, paths)| {
                paths
                    .iter()
                    .find_map(|path| std::fs::read(path).ok().filter(|font| !font.is_empty()))
                    .map(|bytes| ((*family).to_owned(), bytes))
            })
            .collect()
    }

    fn apply_ime(&self, update: &engine::session::ImeUpdate) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        let update = update.clone();
        on_main(Box::new(move || {
            // SAFETY: the address is the content view stored on the main thread, and this closure runs there too.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_ime_update(&update);
        }));
    }

    fn inspect_finished(&self) {
        wake_main();
    }

    fn request_exit(&self) {
        wake_main();
    }
}

/// A script session. The shell paints the layer tree that session records.
pub enum Launch {
    Script {
        name: String,
        source: Vec<u8>,
        bytecode: bool,
        modules: Option<Vec<String>>,
    },
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
    native: crate::native::Host,
    options: WindowOptions,
) -> Result<(), RunError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(RunError::Window(anyhow::anyhow!(
            "AppKit must start on the main thread"
        )));
    };
    let vsync = Arc::new(DisplayLink::new().map_err(RunError::Window)?);
    let inspect = std::env::var_os("IMPASTO_INSPECT").map(PathBuf::from);
    let dump = std::env::var_os("IMPASTO_DUMP")
        .map(PathBuf::from)
        .or_else(|| inspect.as_ref().map(|dir| dir.join("frame.png")));
    let exit_code = Arc::new(AtomicI32::new(0));
    let Launch::Script {
        name,
        source,
        bytecode,
        modules,
    } = launch;
    #[cfg(debug_assertions)]
    let development = if bytecode {
        None
    } else {
        development_config().map_err(|error| RunError::Window(error.into()))?
    };
    let devtools_source_map = load_source_map(&name);
    let inspect_dir = std::env::var_os("IMPASTO_INSPECT").map(PathBuf::from);
    let diagnostics = modules
        .as_ref()
        .is_none_or(|enabled| enabled.iter().any(|name| name == "devtools"));
    let factory: embedder::SessionFactory = Box::new(move |open, host| {
        let host = modules
            .as_ref()
            .map_or_else(|| host.clone(), |modules| host.clone().restrict(modules));
        if bytecode {
            // SAFETY: `source` is the app file the host selected. A bytecode
            // launch is `app.bin` written by `impasto build` with this
            // toolchain's `compile`. It is not network input. The envelope
            // does not prove the payload is valid bytecode.
            unsafe {
                engine::js::open_trusted_bytecode(&source, &name, open.shaper, open.log, host)
            }
        } else {
            engine::js::open_program(&source, &name, open.shaper, open.log, host)
        }
        .map_err(|error| match error {
            engine::js::Error::Runtime => SessionError::Runtime,
            engine::js::Error::Script(text) => {
                tracing::error!(target: "impasto::script", "{text}");
                SessionError::Script(text)
            }
        })
    });
    // SAFETY: The factory keeps the shaper, VM and log callback in the JS session.
    let embedder_launch = unsafe { EmbedderLaunch::migrating(factory) };
    let dump_for_surface = dump;
    let exit_for_surface = Arc::clone(&exit_code);
    let ime_view = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
    let decoder = Arc::new(PngDecoder { directory: assets });
    let launch_url = engine::session::shared(initial_url.clone());
    let devtools = diagnostics.then(|| Arc::new(embedder::DiagnosticsHub::new()));
    fn log_directory() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let dir = PathBuf::from(home).join("Library/Logs/impasto");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }
    let boot = Boot {
        font_family: FONT_FAMILY.to_string(),
        launch: embedder_launch,
        services: Box::new(MacServices {
            font: font.to_vec(),
            ime_view: Arc::clone(&ime_view),
        }),
        vsync: Arc::clone(&vsync) as Arc<dyn embedder::Vsync>,
        decoder,
        modules: Vec::new(),
        plugin_services: embedder::PluginServices {
            initial_lifecycle: Default::default(),
            native_channel: native.channel(),
            native_events: native.events(),
            paths: embedder::AppPaths::new(
                sandbox_directory(objc2_foundation::NSSearchPathDirectory::DocumentDirectory),
                sandbox_directory(objc2_foundation::NSSearchPathDirectory::CachesDirectory),
                Some(std::env::temp_dir()),
            )
            .with_data_dir(sandbox_directory(
                objc2_foundation::NSSearchPathDirectory::ApplicationSupportDirectory,
            ))
            .with_downloads_dir(sandbox_directory(
                objc2_foundation::NSSearchPathDirectory::DownloadsDirectory,
            )),
        },
        platform_runner: Some(embedder::PlatformTaskRunner::new(
            Arc::new(dispatch_main),
            dispatch_main_after,
            || MainThreadMarker::new().is_some(),
        )),
        launch_url: Arc::clone(&launch_url),
        open_raster: Box::new(|fonts| {
            raster::Rasterizer::create(raster::RasterOptions::default(), fonts)
        }),
        open_surface: Box::new(move |raster, target| {
            Ok(Box::new(MetalSurface::open(
                raster,
                target,
                dump_for_surface.clone(),
                Arc::clone(&exit_for_surface),
            )?))
        }),
        dump_list,
        inspect,
        devtools,
        devtools_source_map,
        inspect_dir,
        log_dir: log_directory(),
    };
    let startup_error = Arc::new(std::sync::Mutex::new(None));
    let link = embedder::start_async(move || Ok(boot), {
        let startup_error = Arc::clone(&startup_error);
        move |result| {
            if let Err(error) = result
                && !matches!(error, BootError::Cancelled)
            {
                eprintln!("impasto: {error}");
                *startup_error
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(error);
                wake_main();
            }
        }
    })
    .map_err(RunError::Boot)?;

    #[cfg(debug_assertions)]
    let _development = development
        .map(|(dev, id, socket)| {
            embedder::dev_channel::Server::connect_unix(dev, link.poster(), id, socket)
        })
        .transpose()
        .map_err(|error| RunError::Window(error.into()))?;

    let app = NSApplication::sharedApplication(mtm);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    menu::install(&app, mtm, &options.title);
    let frame = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(options.width, options.height),
    );
    // SAFETY: the style is titled, closable, miniaturizable, and resizable, with buffered backing.
    let window = unsafe {
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
    vsync.attach(&window, mtm);
    window.setTitle(&NSString::from_str(&options.title));
    window.setContentMinSize(CGSize::new(
        options.min_width.unwrap_or(0.0),
        options.min_height.unwrap_or(0.0),
    ));
    window.center();
    if let Some(name) = &options.autosave_name {
        let name = NSString::from_str(name);
        let _ = window.setFrameAutosaveName(&name);
        let _ = window.setFrameUsingName(&name);
        if options.size_overridden {
            window.setContentSize(frame.size);
        }
    }
    window.setAcceptsMouseMovedEvents(true);
    // SAFETY: `HostState` keeps this window retained until the process leaves `run`.
    unsafe { window.setReleasedWhenClosed(false) };

    let scale = point_scale(&window, mtm);
    let layer = CAMetalLayer::layer();
    layer.setContentsScale(scale);
    layer.setFrame(frame);
    layer.setDrawableSize(CGSize::new(
        frame.size.width * scale,
        frame.size.height * scale,
    ));

    let view = ContentView::new(mtm, frame, link.poster());
    ime_view.store(Retained::as_ptr(&view).cast_mut().cast(), Ordering::Release);
    view.setWantsLayer(true);
    let backing: &CALayer = layer.as_ref();
    view.setLayer(Some(backing));
    window.setContentView(Some(&*view));

    let host = Host::new(
        mtm,
        HostState {
            window,
            layer,
            surface: Cell::new(None),
            link,
            initial_url,
            native: RefCell::new(Some(native)),
            detached: Cell::new(false),
            launched: Cell::new(false),
        },
    );
    host.ivars()
        .window
        .setDelegate(Some(ProtocolObject::from_ref(&*host)));
    app.setDelegate(Some(ProtocolObject::from_ref(&*host)));
    watch_appearance(&host);
    app.run();
    host.detach();
    if let Some(error) = startup_error
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    {
        return Err(RunError::Boot(error));
    }
    if exit_code.load(Ordering::SeqCst) != 0 {
        return Err(RunError::Dump);
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn development_config()
-> std::io::Result<Option<(Arc<embedder::dev_channel::Development>, String, PathBuf)>> {
    let (Ok(token), Ok(id), Some(socket)) = (
        std::env::var("IMPASTO_DEV_SESSION"),
        std::env::var("IMPASTO_DEV_APP_ID"),
        std::env::var_os("IMPASTO_DEV_SOCKET"),
    ) else {
        return Ok(None);
    };
    let caches = sandbox_directory(objc2_foundation::NSSearchPathDirectory::CachesDirectory)
        .ok_or_else(|| std::io::Error::other("development caches are unavailable"))?;
    let app_caches = caches.join("Impasto").join(id.replace(['/', '\\'], "_"));
    let dev = embedder::dev_channel::prepare(
        token,
        &app_caches,
        embedder::dev_channel::Logging {
            terminal: true,
            verbose: std::env::var("IMPASTO_DEV_VERBOSE").as_deref() == Ok("1"),
        },
    )?;
    Ok(Some((dev, id, PathBuf::from(socket))))
}

fn watch_appearance(host: &Host) {
    let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
    // SAFETY: the selector is `themeChanged:` on Host, and the notification name is a constant.
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

impl embedder::ImageDecoder for PngDecoder {
    fn read_packaged_asset(&self, relative: &str, limit: usize) -> Result<Arc<[u8]>, String> {
        embedder::read_packaged_file(&self.directory, relative, limit)
    }

    fn decode(&self, source: &str) -> Result<embedder::ImageHandle, embedder::ImageError> {
        self.decode_at(source, 1)
    }

    fn decode_at(
        &self,
        source: &str,
        scale_bucket: u8,
    ) -> Result<embedder::ImageHandle, embedder::ImageError> {
        self.decode_sized(source, scale_bucket, None)
    }

    fn decode_sized(
        &self,
        source: &str,
        scale_bucket: u8,
        target: Option<embedder::DecodeTarget>,
    ) -> Result<embedder::ImageHandle, embedder::ImageError> {
        let Some(path) = crate::assets::resolve_asset_path(&self.directory, source, scale_bucket)
        else {
            return Err(embedder::ImageError::Missing);
        };
        let rgba = crate::pixels::load_rgba_png(&path, target)
            .map_err(|error| embedder::ImageError::Failed(error.to_string()))?;
        Ok(image_handle(source, rgba))
    }

    fn decode_bytes(
        &self,
        source: &str,
        bytes: &[u8],
    ) -> Result<embedder::ImageHandle, embedder::ImageError> {
        self.decode_bytes_sized(source, bytes, None)
    }

    fn decode_bytes_sized(
        &self,
        source: &str,
        bytes: &[u8],
        target: Option<embedder::DecodeTarget>,
    ) -> Result<embedder::ImageHandle, embedder::ImageError> {
        let rgba = crate::pixels::load_rgba_bytes(bytes, target)
            .map_err(|error| embedder::ImageError::Failed(error.to_string()))?;
        Ok(image_handle(source, rgba))
    }
}

fn image_handle(source: &str, rgba: crate::pixels::Rgba) -> embedder::ImageHandle {
    embedder::ImageHandle {
        source: Arc::from(source),
        generation: 0,
        size: [rgba.width, rgba.height],
        pixels: Arc::from(rgba.pixels),
        gpu: None,
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
    use engine::session::{Brightness, Key};

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
mod scroll_tests {
    use super::{NSEventPhase, ScrollPhase, scroll_phase_of};

    #[test]
    fn appkit_contact_and_momentum_keep_separate_phases() {
        use NSEventPhase as Native;
        let empty = Native::empty();
        for (contact, momentum, expected) in [
            (empty, empty, ScrollPhase::Discrete),
            (Native::MayBegin, empty, ScrollPhase::MayBegin),
            (Native::Began, empty, ScrollPhase::Began),
            (Native::Stationary, empty, ScrollPhase::Changed),
            (Native::Ended, empty, ScrollPhase::Ended),
            (Native::Cancelled, empty, ScrollPhase::Cancelled),
            (empty, Native::Began, ScrollPhase::MomentumBegan),
            (empty, Native::Stationary, ScrollPhase::MomentumChanged),
            (empty, Native::Changed, ScrollPhase::MomentumChanged),
            (empty, Native::Ended, ScrollPhase::MomentumEnded),
            (empty, Native::Cancelled, ScrollPhase::MomentumEnded),
        ] {
            assert_eq!(scroll_phase_of(contact, momentum), expected);
        }
    }
}

#[cfg(test)]
mod url_tests {
    use super::{Command, nonempty_url, resize_command, skip_launch_duplicate};

    #[test]
    fn resize_with_surface_posts_surface_changed_with_scaled_pixels() {
        let id = embedder::SurfaceId::fresh();
        let command = resize_command(Some(id), 800.0, 600.0, 2.0);
        assert!(matches!(
            command,
            Command::SurfaceChanged {
                id: changed_id,
                pixels,
                ..
            } if changed_id == id && pixels.width == 1600 && pixels.height == 1200
        ));
        assert!(matches!(
            resize_command(None, 800.0, 600.0, 2.0),
            Command::Metrics(_)
        ));
    }

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

fn sandbox_directory(directory: objc2_foundation::NSSearchPathDirectory) -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSSearchPathDomainMask};
    let url = NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            directory,
            NSSearchPathDomainMask::UserDomainMask,
            None,
            true,
        )
        .ok()?;
    url.path().map(|path| PathBuf::from(path.to_string()))
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
    // SAFETY: `NSPasteboard` was looked up; `generalPasteboard` returns the process pasteboard.
    let pasteboard: *mut AnyObject = unsafe { msg_send![class, generalPasteboard] };
    if pasteboard.is_null() {
        return String::new();
    }
    let kind = NSString::from_str(kind);
    // SAFETY: `pasteboard` is non-null and `kind` is the UTI string for this read.
    let value: *mut AnyObject = unsafe { msg_send![pasteboard, stringForType: &*kind] };
    if value.is_null() {
        return String::new();
    }
    // SAFETY: `value` is the non-null NSString returned for this pasteboard type.
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

pub(crate) fn dispatch_main(work: Box<dyn FnOnce() + Send>) {
    let context = Box::into_raw(Box::new(work));
    // SAFETY: `_dispatch_main_q` is the main queue, and `context` is the boxed closure the callback frees.
    unsafe {
        let queue = std::ptr::addr_of!(_dispatch_main_q).cast_mut().cast();
        dispatch_async_f(queue, context.cast(), run_on_main);
    }
}

fn dispatch_main_after(delay: std::time::Duration, work: embedder::Task) {
    let nanos = i64::try_from(delay.as_nanos()).unwrap_or(i64::MAX);
    let context = Box::into_raw(Box::new(work));
    // SAFETY: Dispatch owns the boxed closure until its main-queue callback.
    unsafe {
        let queue = std::ptr::addr_of!(_dispatch_main_q).cast_mut().cast();
        dispatch_after_f(dispatch_time(0, nanos), queue, context.cast(), run_on_main);
    }
}

pub(crate) fn on_main(work: embedder::Task) {
    if MainThreadMarker::new().is_some() {
        work();
    } else {
        dispatch_main(work);
    }
}

// SAFETY: `dispatch_async_f` calls this once with the pointer passed as `context`.
unsafe extern "C" fn run_on_main(context: *mut c_void) {
    // SAFETY: `context` is the boxed closure from `dispatch_main`, and this call takes it once.
    let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
    work();
}

// SAFETY: these are the libdispatch symbols used to hop to the main queue.
unsafe extern "C" {
    static _dispatch_main_q: u8;
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn dispatch_after_f(
        when: u64,
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        // SAFETY: the callback is `run_on_main`, which frees `context` exactly once.
        work: unsafe extern "C" fn(*mut c_void),
    );
}
