use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use dispatch2::{DispatchQueue, DispatchTime};
use embedder::{
    Boot, BootError, Command, FrameToken, Lifecycle, Link, Restoration, Services, SurfaceId,
    SurfaceTarget, Vsync, WindowSession,
};
use engine::input::{Button, Buttons, Modifiers, Phase, PointerEvent, PointerId, PointerKind};
use engine::session::{Brightness, FrameTime, Insets, Metrics, PlatformSettings, WindowDisplay};
use objc2::MainThreadMarker;
use raster::Rasterizer;

use crate::bundle;
use crate::surface::{LayerLease, MetalSurface, SurfaceGate};

#[path = "compositor.rs"]
pub(crate) mod compositor;
#[path = "input.rs"]
mod input;

#[repr(u32)]
pub(crate) enum Event {
    Ready = 1,
    Failed = 2,
    FrameRequested = 3,
    FrameCancelled = 4,
    FrameSubmitted = 5,
    FramePresented = 6,
    Closed = 7,
    Ime = 8,
    Semantics = 9,
    BackEnabled = 10,
    PlatformViews = 11,
    CompositorLayers = 12,
    ComposedFrame = 13,
    #[cfg(debug_assertions)]
    SourceUpdated = 14,
    ServiceRequest = 20,
    RestorationChanged = 21,
    BackUnhandled = 22,
}

type EventFn = extern "C" fn(*mut c_void, u32, u64, *const u8, usize);
type ReleaseFn = extern "C" fn(*mut c_void);

/// The foreign caller retains its context once per host. Its callbacks accept
/// concurrent calls and marshal the borrowed event payload to UIKit themselves.
pub(crate) struct Events {
    context: usize,
    send: Option<EventFn>,
    release: Option<ReleaseFn>,
}

impl Events {
    pub(crate) fn send(&self, event: Event, value: u64, message: Option<&str>) {
        if let Some(send) = self.send {
            let (bytes, length) =
                message.map_or((ptr::null(), 0), |text| (text.as_ptr(), text.len()));
            send(
                ptr::with_exposed_provenance_mut(self.context),
                event as u32,
                value,
                bytes,
                length,
            );
        }
    }

    fn failed(&self, error: &BootError) {
        if !matches!(error, BootError::Cancelled) {
            self.send(
                Event::Failed,
                u64::from(error.code()),
                Some(&error.to_string()),
            );
        }
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        if let Some(release) = self.release {
            release(ptr::with_exposed_provenance_mut(self.context));
        }
    }
}

type FrameSink = Arc<dyn Fn(FrameToken, FrameTime) + Send + Sync>;

struct DisplayLink {
    events: Arc<Events>,
    sink: OnceLock<FrameSink>,
}

impl DisplayLink {
    fn frame(&self, token: FrameToken, seconds: f64, target_seconds: f64) {
        if let Some(sink) = self.sink.get()
            && seconds.is_finite()
            && target_seconds.is_finite()
            && target_seconds > seconds
        {
            sink(
                token,
                FrameTime {
                    vsync_ms: seconds * 1_000.0,
                    target_ms: target_seconds * 1_000.0,
                    interval_ms: (target_seconds - seconds) * 1_000.0,
                    mono_ms: 0.0,
                },
            );
        }
    }
}

impl Vsync for DisplayLink {
    fn bind(&self, sink: FrameSink) {
        let _ = self.sink.set(sink);
    }

    fn request_frame(&self, token: FrameToken) {
        self.events.send(Event::FrameRequested, token.0, None);
    }

    fn cancel(&self) {
        self.events.send(Event::FrameCancelled, 0, None);
    }
}

struct HostServices {
    font_family: String,
    font: Vec<u8>,
    events: Arc<Events>,
    surface: Cell<u64>,
    ime: RefCell<Option<String>>,
    semantics: RefCell<Option<String>>,
    back_enabled: Cell<bool>,
    restoration: Restoration,
    window_group: WindowSession,
    compositor: Arc<compositor::Compositor>,
}

impl Services for HostServices {
    fn font(&self, family: &str) -> Option<&[u8]> {
        (family == self.font_family).then_some(&self.font)
    }

    fn restoration(&self) -> Option<Restoration> {
        Some(self.restoration.clone())
    }

    fn window_session(&self) -> WindowSession {
        self.window_group.clone()
    }

    fn websocket_transport(&self) -> Option<Arc<dyn embedder::WebSocketTransport>> {
        Some(bundle::websocket_transport())
    }

    fn surface_attached(&self, surface: SurfaceId) {
        self.surface.set(surface.0);
        if let Some(ime) = self.ime.borrow().as_deref() {
            self.events.send(Event::Ime, surface.0, Some(ime));
        }
        if let Some(semantics) = self.semantics.borrow().as_deref() {
            self.events
                .send(Event::Semantics, surface.0, Some(semantics));
        }
        self.back_enabled(self.back_enabled.get());
    }

    fn apply_ime(&self, update: &engine::session::ImeUpdate) {
        let message = input::ime_message(update);
        self.events
            .send(Event::Ime, self.surface.get(), Some(&message));
        self.ime.replace(Some(message));
    }

    fn semantics(&self, nodes: &[embedder::ProjectedNode]) {
        let message = input::semantics_message(nodes);
        self.events
            .send(Event::Semantics, self.surface.get(), Some(&message));
        self.semantics.replace(Some(message));
    }

    fn semantics_include_platform_views(&self) -> bool {
        true
    }

    fn back_enabled(&self, enabled: bool) {
        self.back_enabled.set(enabled);
        self.events
            .send(Event::BackEnabled, u64::from(enabled), None);
    }

    fn unhandled_back(&self) {
        self.events
            .send(Event::BackUnhandled, self.surface.get(), None);
    }

    fn request_exit(&self) {
        self.unhandled_back();
    }

    fn platform_views(&self, surface: SurfaceId, views: &[engine::tree::PlatformViewPlacement]) {
        if self.compositor.configured(surface.0) {
            return;
        }
        let message = input::platform_views_value(views).to_string();
        self.events
            .send(Event::PlatformViews, surface.0, Some(&message));
    }

    #[cfg(debug_assertions)]
    fn source_updated(&self, revision: &str, status: Result<(), &str>) {
        if let Ok(revision) = revision.parse() {
            self.events
                .send(Event::SourceUpdated, revision, status.err());
        }
    }
}

pub struct ImpastoIOSHost {
    link: Link,
    clock: Arc<DisplayLink>,
    gate: Arc<SurfaceGate>,
    // Only UIKit adds/removes entries. The exit callback keeps the registry
    // alive across an asynchronous close, including startup cancellation.
    layers: Arc<Mutex<HashMap<SurfaceId, LayerLease>>>,
    viewport: RefCell<Option<Metrics>>,
    restoration: Restoration,
    platform: Arc<bundle::PlatformServices>,
    compositor: Arc<compositor::Compositor>,
}

pub struct ImpastoIOSWindowGroup(WindowSession);

impl ImpastoIOSHost {
    fn start(
        root: PathBuf,
        initial_url: Option<String>,
        saved: Option<String>,
        window_group: WindowSession,
        events: Arc<Events>,
    ) -> Result<Self, BootError> {
        let gate = Arc::new(SurfaceGate::default());
        let layers = Arc::new(Mutex::new(HashMap::new()));
        let clock = Arc::new(DisplayLink {
            events: Arc::clone(&events),
            sink: OnceLock::new(),
        });
        let launch_url = engine::session::shared(initial_url);
        let boot_gate = Arc::clone(&gate);
        let boot_events = Arc::clone(&events);
        let boot_clock = Arc::clone(&clock);
        let compositor = Arc::new(compositor::Compositor::default());
        let boot_compositor = Arc::clone(&compositor);
        let ready_events = Arc::clone(&events);
        let exit_layers = Arc::clone(&layers);
        let started = Arc::new(AtomicBool::new(false));
        let exit_started = Arc::clone(&started);
        let restore_events = Arc::clone(&events);
        let restoration = Restoration::new_notifying(saved, move || {
            restore_events.send(Event::RestorationChanged, 0, None);
        });
        let boot_restoration = restoration.clone();
        let platform_events = Arc::clone(&events);
        let platform = Arc::new(bundle::PlatformServices::new(move |id, request| {
            platform_events.send(Event::ServiceRequest, id, Some(request));
        }));
        let boot_platform = Arc::clone(&platform);
        let exit_platform = Arc::clone(&platform);
        let link = embedder::start_async_with_exit(
            move || {
                let app = bundle::prepare(root, Arc::clone(&launch_url), boot_platform)?;
                Ok(Boot {
                    font_family: app.font_family.clone(),
                    launch: app.launch,
                    services: Box::new(HostServices {
                        font_family: app.font_family,
                        font: app.font,
                        events: Arc::clone(&boot_events),
                        surface: Cell::new(0),
                        ime: RefCell::new(None),
                        semantics: RefCell::new(None),
                        back_enabled: Cell::new(false),
                        restoration: boot_restoration,
                        window_group,
                        compositor: Arc::clone(&boot_compositor),
                    }),
                    vsync: boot_clock,
                    decoder: app.decoder,
                    modules: app.modules,
                    platform_runner: Some(embedder::PlatformTaskRunner::new(
                        Arc::new(|work| DispatchQueue::main().exec_async(work)),
                        |delay, work| {
                            let when =
                                DispatchTime::try_from(delay).unwrap_or(DispatchTime::FOREVER);
                            let _ = DispatchQueue::main().after(when, work);
                        },
                        || MainThreadMarker::new().is_some(),
                    )),
                    thread_topology: embedder::ThreadTopology::configured()?,
                    launch_url,
                    http: app.http,
                    open_raster: Box::new(|fonts| {
                        Rasterizer::create(raster::RasterOptions::default(), fonts)
                    }),
                    open_surface: Box::new(move |raster, target| {
                        Ok(Box::new(MetalSurface::open(
                            raster,
                            target,
                            Arc::clone(&boot_gate),
                            Arc::clone(&boot_events),
                            Arc::clone(&boot_compositor),
                        )?))
                    }),
                    dump_list: false,
                    inspect: None,
                    log_dir: None,
                    devtools: app
                        .devtools
                        .then(|| Arc::new(embedder::DiagnosticsHub::new())),
                    devtools_source_map: None,
                    inspect_dir: None,
                })
            },
            move |result| match result {
                Ok(()) => {
                    started.store(true, Ordering::Release);
                    ready_events.send(Event::Ready, 0, None);
                }
                Err(error) => ready_events.failed(&error),
            },
            move |result| {
                exit_platform.cancel_all();
                // The common host has stopped using all surface pointers.
                exit_layers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clear();
                if exit_started.load(Ordering::Acquire)
                    && let Err(error) = result
                {
                    events.failed(&error);
                }
                events.send(Event::Closed, 0, None);
            },
        )?;
        Ok(Self {
            link,
            clock,
            gate,
            layers,
            viewport: RefCell::new(None),
            restoration,
            platform,
            compositor,
        })
    }

    fn current_surface(&self, id: u64) -> bool {
        id != 0 && self.gate.active.load(Ordering::Acquire) == id
    }

    fn update_viewport(&self, id: u64, metrics: Metrics) {
        self.viewport.replace(Some(metrics));
        self.link.post(Command::SurfaceChanged {
            id: SurfaceId(id),
            pixels: embedder::pixel_size(metrics),
            metrics,
        });
    }
}

/// Used only by synchronous C entry points. A borrowed host never escapes the
/// call. UIKit serializes all calls and close consumes the only foreign handle.
fn with_host<T: Default>(host: *mut ImpastoIOSHost, work: impl FnOnce(&ImpastoIOSHost) -> T) -> T {
    if MainThreadMarker::new().is_none() {
        return T::default();
    }
    // SAFETY: the caller keeps the opaque handle alive on the UIKit thread.
    unsafe { host.as_ref() }.map_or_else(T::default, work)
}

/// Copy a caller-owned C string before returning to the platform.
unsafe fn string(text: *const c_char) -> Option<String> {
    if text.is_null() {
        return None;
    }
    // SAFETY: the caller keeps the NUL-terminated string valid for this call.
    unsafe { CStr::from_ptr(text) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn metrics(width: f32, height: f32, scale: f32, padding: Insets) -> Option<Metrics> {
    if [width, height, scale]
        .into_iter()
        .any(|v| !v.is_finite() || v <= 0.0)
        || [padding.left, padding.top, padding.right, padding.bottom]
            .into_iter()
            .any(|v| !v.is_finite() || v < 0.0)
    {
        return None;
    }
    let metrics = Metrics::from_insets(width, height, scale, padding, Insets::default());
    Some(metrics)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_start(
    bundle_root: *const c_char,
    initial_url: *const c_char,
    context: *mut c_void,
    event: Option<EventFn>,
    release: Option<ReleaseFn>,
) -> *mut ImpastoIOSHost {
    // SAFETY: preserves the original no-restoration, no-group startup contract.
    unsafe {
        impasto_ios_start_restored(
            bundle_root,
            initial_url,
            ptr::null(),
            ptr::null(),
            context,
            event,
            release,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_start_restored(
    bundle_root: *const c_char,
    initial_url: *const c_char,
    restored: *const c_char,
    group: *const ImpastoIOSWindowGroup,
    context: *mut c_void,
    event: Option<EventFn>,
    release: Option<ReleaseFn>,
) -> *mut ImpastoIOSHost {
    let events = Arc::new(Events {
        context: context.expose_provenance(),
        send: event,
        release,
    });
    if MainThreadMarker::new().is_none() || event.is_none() {
        return ptr::null_mut();
    }
    // SAFETY: both C strings are borrowed from the caller for this call.
    let (root, url, saved) =
        unsafe { (string(bundle_root), string(initial_url), string(restored)) };
    let Some(root) = root.filter(|root| !root.is_empty()) else {
        events.failed(&BootError::Platform("缺少应用资源目录".into()));
        return ptr::null_mut();
    };
    // SAFETY: native host owns the optional group through this synchronous call.
    let group = unsafe { group.as_ref() }
        .map(|group| group.0.clone())
        .unwrap_or_default();
    match ImpastoIOSHost::start(PathBuf::from(root), url, saved, group, Arc::clone(&events)) {
        Ok(host) => Box::into_raw(Box::new(host)),
        Err(error) => {
            events.failed(&error);
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_close(host: *mut ImpastoIOSHost) {
    if MainThreadMarker::new().is_some() && !host.is_null() {
        // SAFETY: close consumes the caller's sole handle on the UIKit thread.
        let host = unsafe { Box::from_raw(host) };
        host.platform.cancel_all();
        host.compositor.reset(0);
        host.gate.visible.store(false, Ordering::Release);
        host.gate.active.store(0, Ordering::Release);
        host.link.request_shutdown();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_attach(
    host: *mut ImpastoIOSHost,
    metal_layer: *mut c_void,
    width: f32,
    height: f32,
    scale: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> u64 {
    with_host(host, |host| {
        if host.gate.active.load(Ordering::Acquire) != 0 {
            return 0;
        }
        let Some(metrics) = metrics(
            width,
            height,
            scale,
            Insets {
                left,
                top,
                right,
                bottom,
            },
        ) else {
            return 0;
        };
        // SAFETY: the caller owns a live CAMetalLayer for the entire call.
        let Some(lease) = (unsafe { LayerLease::retain(metal_layer) }) else {
            return 0;
        };
        let id = SurfaceId::fresh();
        let target = SurfaceTarget::new(id, lease.ptr(), metrics);
        host.layers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, lease);
        host.gate.active.store(id.0, Ordering::Release);
        host.compositor.reset(id.0);
        host.viewport.replace(Some(metrics));
        host.link.post(Command::SurfaceReady(target));
        id.0
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_resize(
    host: *mut ImpastoIOSHost,
    surface: u64,
    width: f32,
    height: f32,
    scale: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) {
    with_host(host, |host| {
        if host.current_surface(surface)
            && let Some(mut metrics) = metrics(
                width,
                height,
                scale,
                Insets {
                    left,
                    top,
                    right,
                    bottom,
                },
            )
        {
            if let Some(previous) = *host.viewport.borrow() {
                metrics.view_insets = previous.view_insets;
                metrics.inset_bottom = metrics.padding.bottom.max(metrics.view_insets.bottom);
                metrics.keyboard_occlusion = previous.keyboard_occlusion;
            }
            host.update_viewport(surface, metrics);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_detach(host: *mut ImpastoIOSHost, surface: u64) {
    with_host(host, |host| {
        if host.current_surface(surface) {
            host.gate.active.store(0, Ordering::Release);
            host.viewport.replace(None);
            host.platform.cancel_pending();
            host.compositor.reset(0);
            host.link.begin_destroy(SurfaceId(surface));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_surface_dropped(
    host: *mut ImpastoIOSHost,
    surface: u64,
) -> bool {
    with_host(host, |host| {
        let id = SurfaceId(surface);
        let mut layers = host
            .layers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !layers.contains_key(&id) {
            return true;
        }
        if host.link.poll_dropped(id) {
            layers.remove(&id);
            return true;
        }
        false
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_frame(
    host: *mut ImpastoIOSHost,
    token: u64,
    timestamp_seconds: f64,
    target_seconds: f64,
) {
    with_host(host, |host| {
        host.clock
            .frame(FrameToken(token), timestamp_seconds, target_seconds)
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_lifecycle(host: *mut ImpastoIOSHost, state: u32) {
    with_host(host, |host| {
        let lifecycle = match state {
            0 => Lifecycle::Detached,
            1 => Lifecycle::Resumed,
            2 => Lifecycle::Inactive,
            3 => Lifecycle::Hidden,
            4 => Lifecycle::Paused,
            _ => return,
        };
        host.gate.visible.store(
            matches!(lifecycle, Lifecycle::Resumed | Lifecycle::Inactive),
            Ordering::Release,
        );
        host.link.post(Command::Lifecycle(lifecycle));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_focus(host: *mut ImpastoIOSHost, focused: bool) {
    with_host(host, |host| host.link.post(Command::WindowFocus(focused)));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_invalidate(host: *mut ImpastoIOSHost) {
    with_host(host, |host| host.link.post(Command::Invalidate));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_memory_warning(host: *mut ImpastoIOSHost) {
    with_host(host, |host| host.link.post(Command::MemoryWarning));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_open_url(host: *mut ImpastoIOSHost, url: *const c_char) {
    with_host(host, |host| {
        // SAFETY: url is valid for the duration of this C call.
        if let Some(url) = unsafe { string(url) } {
            host.link.post(Command::LaunchUrl(url));
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_settings(
    host: *mut ImpastoIOSHost,
    locale: *const c_char,
    text_scale: f32,
    dark: bool,
    reduce_motion: bool,
    bold_text: bool,
    high_contrast: bool,
) {
    with_host(host, |host| {
        // SAFETY: locale is valid for the duration of this C call.
        let locale = unsafe { string(locale) }.unwrap_or_default();
        let text_scale = if text_scale.is_finite() && text_scale > 0.0 {
            text_scale
        } else {
            1.0
        };
        host.link.post(Command::Settings(PlatformSettings {
            locale,
            text_scale,
            brightness: if dark {
                Brightness::Dark
            } else {
                Brightness::Light
            },
            reduce_motion,
            bold_text,
            high_contrast,
        }));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_display(host: *mut ImpastoIOSHost, display_id: i32, hz: f32) {
    with_host(host, |host| {
        host.link.post(Command::WindowDisplay(WindowDisplay {
            display_id: (display_id >= 0).then_some(display_id),
            refresh_rate: (hz.is_finite() && hz > 0.0).then_some(hz),
            features: Vec::new(),
        }));
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_pointer(
    host: *mut ImpastoIOSHost,
    pointer: u64,
    kind: u32,
    phase: u32,
    x: f32,
    y: f32,
    seconds: f64,
    button: u32,
    buttons: u32,
    modifiers: u32,
    dx: f32,
    dy: f32,
    zoom: f32,
) {
    with_host(host, |host| {
        let Some(kind) = [
            PointerKind::Touch,
            PointerKind::Mouse,
            PointerKind::Pen,
            PointerKind::Trackpad,
        ]
        .get(kind as usize) else {
            return;
        };
        let Some(phase) = [
            Phase::Down,
            Phase::Move,
            Phase::Up,
            Phase::Cancel,
            Phase::Wheel,
            Phase::Zoom,
        ]
        .get(phase as usize) else {
            return;
        };
        let Some(button) = [
            Button::None,
            Button::Primary,
            Button::Secondary,
            Button::Auxiliary,
        ]
        .get(button as usize) else {
            return;
        };
        if !seconds.is_finite()
            || [x, y, dx, dy, zoom]
                .into_iter()
                .any(|value| !value.is_finite())
        {
            return;
        }
        host.link.post(Command::Pointer(PointerEvent {
            pointer: PointerId(pointer),
            kind: *kind,
            phase: *phase,
            x,
            y,
            time_ms: seconds * 1_000.0,
            button: *button,
            buttons: Buttons::from_bits(u8::try_from(buttons & 0b111).unwrap_or_default()),
            modifiers: Modifiers::from_bits(u8::try_from(modifiers & 0b1111).unwrap_or_default()),
            dx,
            dy,
            zoom,
            pen: None,
        }));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn impasto_ios_window_group_new() -> *mut ImpastoIOSWindowGroup {
    Box::into_raw(Box::new(ImpastoIOSWindowGroup(WindowSession::default())))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_window_group_free(group: *mut ImpastoIOSWindowGroup) {
    if !group.is_null() {
        // SAFETY: this consumes the caller's only ownership of the group.
        drop(unsafe { Box::from_raw(group) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_restoration_copy(
    host: *mut ImpastoIOSHost,
    buffer: *mut u8,
    capacity: usize,
) -> usize {
    with_host(host, |host| {
        let Some(state) = host.restoration.saved_state() else {
            return 0;
        };
        if !buffer.is_null() && capacity >= state.len() {
            // SAFETY: caller owns the writable buffer for capacity bytes.
            unsafe { ptr::copy_nonoverlapping(state.as_ptr(), buffer, state.len()) };
        }
        state.len()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_service_pending(
    host: *mut ImpastoIOSHost,
    request: u64,
) -> bool {
    with_host(host, |host| host.platform.contains(request))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_cancel_native_plugins(host: *mut ImpastoIOSHost) {
    with_host(host, |host| host.platform.cancel_native_plugins());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_service_result(
    host: *mut ImpastoIOSHost,
    request: u64,
    status: u32,
    payload: *const c_char,
) {
    with_host(host, |host| {
        // SAFETY: payload is borrowed from native for this call only.
        host.platform
            .complete(request, status, unsafe { string(payload) });
    });
}

struct TextureOwner {
    value: usize,
    release: Option<ReleaseFn>,
}

impl Drop for TextureOwner {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release(ptr::with_exposed_provenance_mut(self.value));
        }
    }
}

fn external_source(name: &str) -> Option<String> {
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return None;
    }
    Some(format!("external:{name}"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_texture_frame(
    host: *mut ImpastoIOSHost,
    name: *const c_char,
    metal_texture: *mut c_void,
    width: u32,
    height: u32,
    context: *mut c_void,
    release: Option<ReleaseFn>,
) -> bool {
    let owner = Arc::new(TextureOwner {
        value: context.expose_provenance(),
        release,
    });
    with_host(host, |host| {
        // SAFETY: name is borrowed for this C call.
        let Some(source) = (unsafe { string(name) })
            .as_deref()
            .and_then(external_source)
        else {
            return false;
        };
        if metal_texture.is_null()
            || !embedder::pixel_count_ok([width, height])
            || host.gate.active.load(Ordering::Acquire) == 0
        {
            return false;
        }
        let texture = metal_texture.expose_provenance();
        host.link.post(Command::ExternalGpuTexture {
            source,
            frame: Some(Box::new(move |engine| {
                let owner = Arc::clone(&owner);
                // SAFETY: the retained owner keeps the ready producer texture alive.
                unsafe {
                    engine.import_metal_texture(
                        ptr::with_exposed_provenance(texture),
                        [width, height],
                        move || drop(owner),
                    )
                }
            })),
        });
        true
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_texture_unregister(
    host: *mut ImpastoIOSHost,
    name: *const c_char,
) {
    with_host(host, |host| {
        // SAFETY: name is borrowed for this synchronous call.
        if let Some(source) = (unsafe { string(name) })
            .as_deref()
            .and_then(external_source)
        {
            host.link.post(Command::ExternalGpuTexture {
                source,
                frame: None,
            });
        }
    });
}

#[cfg(debug_assertions)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn impasto_ios_update_source(
    host: *mut ImpastoIOSHost,
    revision: u64,
    bytes: *const u8,
    length: usize,
) -> bool {
    with_host(host, |host| {
        // SAFETY: bytes is the caller's readable buffer for length bytes.
        let Some(source) = (unsafe { input::copy_text(bytes, length) }) else {
            return false;
        };
        if source.is_empty() {
            return false;
        }
        host.link.post(Command::UpdateSource {
            revision: revision.to_string(),
            source,
        });
        true
    })
}
