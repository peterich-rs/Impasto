//! AppKit window. The render thread owns the engine and the session.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

use hostshell::{
    Boot, BootError, Command, Launch as ShellLaunch, Link, LogLevel, NativePtr, Poster, Services,
    SessionError, SurfaceTarget, metrics,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, available, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSEvent, NSScreen, NSTextInputClient, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSArray, NSAttributedString, NSNotFound, NSNotification, NSPoint, NSRange, NSRect, NSString,
    NSUInteger,
};
use session::{InputEvent, Key};

use crate::text_client::Marked;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLPixelFormat};
use objc2_quartz_core::{CALayer, CAMetalLayer};

use crate::pixels::Rgba;
use crate::pointer::pointer_event;
use crate::surface::MetalSurface;
use crate::vsync::DisplayLink;

const CONTENT_WIDTH: f64 = 360.0;
const CONTENT_HEIGHT: f64 = 640.0;
const FONT_FAMILY: &str = "UIFWSans";

struct ContentIvars {
    poster: Poster,
    wants_ime: Cell<bool>,
    marked: RefCell<Marked>,
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
            NSRange {
                location: self.ivars().marked.borrow().caret_utf16() as NSUInteger,
                length: 0,
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
            self.screen_bounds()
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn characterIndexForPoint(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }

    impl ContentView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            self.ivars().wants_ime.get()
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.enqueue(core::Phase::Down, event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.enqueue(core::Phase::Move, event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.enqueue(core::Phase::Up, event);
        }
    }
);

impl ContentView {
    fn new(mtm: MainThreadMarker, frame: CGRect, poster: Poster) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ContentIvars {
            poster,
            wants_ime: Cell::new(false),
            marked: RefCell::new(Marked::default()),
        });
        // SAFETY: `initWithFrame:` is NSView's designated initializer. The ivars are
        // already installed, and `frame` is a finite content rectangle.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    fn enqueue(&self, phase: core::Phase, event: &NSEvent) {
        let local = self.convertPoint_fromView(event.locationInWindow(), None);
        let height = self.bounds().size.height;
        self.ivars().poster.post(Command::Pointer(pointer_event(
            phase,
            local.x,
            local.y,
            height,
            event.timestamp() * 1000.0,
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
        let events = match name.as_str() {
            "insertNewline:" => vec![InputEvent::Key(Key::Enter)],
            "insertTab:" => vec![InputEvent::Key(Key::Tab)],
            "deleteBackward:" => self.ivars().marked.borrow_mut().backspace(),
            "moveLeft:" => vec![InputEvent::Key(Key::Left)],
            "moveRight:" => vec![InputEvent::Key(Key::Right)],
            "moveUp:" => vec![InputEvent::Key(Key::Up)],
            "moveDown:" => vec![InputEvent::Key(Key::Down)],
            _ => {
                eprintln!("uifw text command ignored: {name}");
                return;
            }
        };
        self.post_edits(events);
    }

    fn screen_bounds(&self) -> NSRect {
        let bounds = self.bounds();
        let in_window = self.convertRect_toView(bounds, None);
        match self.window() {
            Some(window) => window.convertRectToScreen(in_window),
            None => in_window,
        }
    }

    fn apply_ime(&self, enabled: bool) {
        self.ivars().wants_ime.set(enabled);
        let Some(window) = self.window() else {
            return;
        };
        if enabled {
            let _: bool = window.makeFirstResponder(Some(self));
        } else {
            let _: bool = window.makeFirstResponder(None);
        }
    }

    fn apply_ime_text(&self, text: String) {
        self.ivars().marked.borrow_mut().replace_committed(text);
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

struct HostState {
    window: Retained<NSWindow>,
    layer: Retained<CAMetalLayer>,
    link: Link,
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
            self.finish_launching();
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            let _ = self.ivars().link.shutdown_and_join();
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
            layer: NativePtr(layer),
            metrics: metrics(CONTENT_WIDTH as f32, CONTENT_HEIGHT as f32, scale as f32),
        }));
        println!("uifw host ready");
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
}

struct MacServices {
    font: Vec<u8>,
    assets: PathBuf,
    images: Mutex<HashMap<String, Arc<Rgba>>>,
    ime_view: Arc<AtomicPtr<c_void>>,
}

impl MacServices {
    fn cached<'a>(&'a self, source: &str) -> Option<&'a Rgba> {
        let mut images = self
            .images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !images.contains_key(source) {
            let path = asset_png(&self.assets, source)?;
            let rgba = crate::pixels::load_rgba_png(&path).ok()?;
            images.insert(source.to_string(), Arc::new(rgba));
        }
        let ptr = Arc::as_ptr(images.get(source)?);
        // SAFETY: the map only inserts. The `Rgba` allocation lives until `MacServices`
        // drops, which is after `submit` has finished reading these pixels.
        Some(unsafe { &*ptr })
    }
}

impl Services for MacServices {
    fn font(&self, family: &str) -> Option<&[u8]> {
        (family == FONT_FAMILY).then_some(self.font.as_slice())
    }

    fn image<'a>(&'a self, source: &str) -> Option<engine::ImagePixels<'a>> {
        let Some(rgba) = self.cached(source) else {
            self.log(LogLevel::Error, &format!("unresolved image {source}"));
            return None;
        };
        Some(engine::ImagePixels {
            width: rgba.width,
            height: rgba.height,
            rgba: &rgba.pixels,
        })
    }

    fn log(&self, level: LogLevel, message: &str) {
        match level {
            LogLevel::Error => eprintln!("{message}"),
            LogLevel::Info => println!("{message}"),
        }
    }

    fn set_ime(&self, enabled: bool) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        dispatch_main(Box::new(move || {
            // SAFETY: the address is the content view, stored on the main thread before
            // the run loop, and this closure also runs on the main thread. The window
            // retains the view for the process lifetime.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_ime(enabled);
        }));
    }

    fn set_ime_text(&self, text: &str) {
        let view = self.ime_view.load(Ordering::Acquire) as usize;
        if view == 0 {
            return;
        }
        let text = text.to_owned();
        dispatch_main(Box::new(move || {
            // SAFETY: same view address as `set_ime`, used only on the main thread.
            let view = unsafe { &*(view as *const ContentView) };
            view.apply_ime_text(text);
        }));
    }
}

/// What the window paints. The fixture path submits once. A script is a session.
pub enum Launch {
    Fixture(protocol::DrawList),
    Script(String),
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

pub fn run(launch: Launch, assets: PathBuf, font: &[u8]) -> Result<(), RunError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(RunError::Window(anyhow::anyhow!(
            "AppKit must start on the main thread"
        )));
    };
    let vsync = DisplayLink::new().map_err(RunError::Window)?;
    let dump = std::env::var_os("UIFW_DUMP").map(PathBuf::from);
    let exit_code = Arc::new(AtomicI32::new(0));
    let exit_for_stop = Arc::clone(&exit_code);
    let on_stop: Arc<dyn Fn(i32) + Send + Sync> = Arc::new(move |code| {
        exit_for_stop.store(code, Ordering::SeqCst);
        wake_main();
    });
    let shell_launch = match launch {
        Launch::Fixture(list) => ShellLaunch::Fixture(list),
        Launch::Script(script) => ShellLaunch::Session(Box::new(move |slot| {
            quickjs::open(&script, slot.shaper(), slot.log_hook()).map_err(|error| match error {
                quickjs::Error::NotLinked => SessionError::NotLinked,
                quickjs::Error::Runtime => SessionError::Runtime,
                quickjs::Error::Script(text) => SessionError::Script(text),
            })
        })),
    };
    let dump_for_surface = dump.clone();
    let on_stop_for_surface = Arc::clone(&on_stop);
    let images = preload_assets(&assets);
    let ime_view = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
    let link = hostshell::start(Boot {
        font_family: FONT_FAMILY.to_string(),
        launch: shell_launch,
        services: Box::new(MacServices {
            font: font.to_vec(),
            assets,
            images: Mutex::new(images),
            ime_view: Arc::clone(&ime_view),
        }),
        vsync: Arc::new(vsync),
        open_surface: Box::new(move |_engine, target| {
            Ok(Box::new(MetalSurface::new(
                target.layer,
                dump_for_surface.clone(),
                Arc::clone(&on_stop_for_surface),
            )))
        }),
    })
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
    window.setTitle(&NSString::from_str("uifw counter"));
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
        },
    );
    host.ivars()
        .window
        .setDelegate(Some(ProtocolObject::from_ref(&*host)));
    app.setDelegate(Some(ProtocolObject::from_ref(&*host)));
    app.run();
    if exit_code.load(Ordering::SeqCst) != 0 {
        return Err(RunError::Dump);
    }
    Ok(())
}

fn preload_assets(directory: &Path) -> HashMap<String, Arc<Rgba>> {
    let mut images = HashMap::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return images;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Ok(rgba) = crate::pixels::load_rgba_png(&path) else {
            continue;
        };
        images.insert(format!("asset:{stem}"), Arc::new(rgba));
    }
    images
}

fn asset_png(directory: &Path, source: &str) -> Option<PathBuf> {
    let name = source.strip_prefix("asset:")?;
    if name.is_empty() {
        return None;
    }
    let mut path = directory.to_path_buf();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return None;
        }
        path.push(part);
    }
    path.set_extension("png");
    Some(path)
}

fn point_scale(window: &NSWindow, mtm: MainThreadMarker) -> f64 {
    let screen = window.screen().or_else(|| NSScreen::mainScreen(mtm));
    let factor = screen
        .map(|screen| screen.backingScaleFactor())
        .unwrap_or(0.0);
    if factor > 0.0 { factor } else { 2.0 }
}

#[cfg(test)]
mod asset_tests {
    use std::path::Path;

    use super::asset_png;

    #[test]
    fn logo_stays_inside_the_asset_directory() {
        let Some(path) = asset_png(Path::new("/assets"), "asset:logo") else {
            panic!("logo");
        };
        assert_eq!(path, Path::new("/assets/logo.png"));
    }

    #[test]
    fn parent_segments_are_rejected() {
        assert!(asset_png(Path::new("/assets"), "asset:../logo").is_none());
        assert!(asset_png(Path::new("/assets"), "logo").is_none());
        assert!(asset_png(Path::new("/assets"), "asset:").is_none());
    }
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
