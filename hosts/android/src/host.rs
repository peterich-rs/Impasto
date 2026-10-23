//! GameActivity loop. Pointers, insets, and pause are posted. The window is released
//! only after the render thread drops the surface.

use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, CString, c_char};
use std::sync::{Arc, Mutex, OnceLock};

use android_activity::input::{
    ImeOptions, InputEvent, InputType, KeyAction, MotionAction, TextInputAction, TextInputState,
    TextSpan,
};
use android_activity::{AndroidApp, AndroidAppWaker, InputStatus, MainEvent, PollEvent};
use hostshell::{
    Boot, Command, Launch, LogLevel, NativePtr, Services, SessionError, SurfaceTarget, Vsync,
};
use ndk::asset::AssetManager;
use ndk::native_window::NativeWindow;
use session::{InputEvent as EditEvent, Key, Metrics};

use crate::decode::{self, Rgba};
use crate::insets::{content_insets, density_scale, insets_in_points, scene_point};
use crate::motion::{self, Action};
use crate::surface::VulkanSurface;
use crate::textinput::{self, Edit, KeyOutcome, Snapshot};
use crate::vsync::FrameClock;

const FONT_FAMILY: &str = "UIFWSans";
const FONT_PATH: &CStr = c"fonts/UIFWSans.ttf";
const SCRIPT_PATH: &CStr = c"counter.js";

unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
}

struct AndroidServices {
    assets: Arc<AssetManager>,
    font: OnceLock<Vec<u8>>,
    images: Mutex<HashMap<String, Arc<Rgba>>>,
    ime: Arc<ImeMailbox>,
    waker: AndroidAppWaker,
}

enum ImeCommand {
    Show,
    Hide,
    Text(String),
}

struct ImeMailbox {
    commands: Mutex<VecDeque<ImeCommand>>,
}

impl ImeMailbox {
    fn push(&self, command: ImeCommand) {
        self.commands
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(command);
    }

    fn drain(&self) -> VecDeque<ImeCommand> {
        std::mem::take(
            &mut *self
                .commands
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
}

impl AndroidServices {
    fn font_bytes(&self) -> Option<&[u8]> {
        let bytes = self
            .font
            .get_or_init(|| decode::read_bytes(&self.assets, FONT_PATH).unwrap_or_default());
        if bytes.is_empty() {
            None
        } else {
            Some(bytes.as_slice())
        }
    }

    fn cached<'a>(&'a self, source: &str) -> Option<&'a Rgba> {
        let mut images = self
            .images
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !images.contains_key(source) {
            let Some(path) = asset_image_path(source) else {
                log_line(LogLevel::Error, &format!("bad asset name {source}"));
                return None;
            };
            let bytes = match load_asset(&self.assets, &path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    log_line(
                        LogLevel::Error,
                        &format!("asset {} failed: {error}", path.to_string_lossy()),
                    );
                    return None;
                }
            };
            let rgba = match decode::decode_rgba(&bytes) {
                Ok(rgba) => rgba,
                Err(error) => {
                    let magic: String = bytes
                        .iter()
                        .take(8)
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    log_line(
                        LogLevel::Error,
                        &format!(
                            "decode failed {} ({} bytes {magic}): {error}",
                            path.to_string_lossy(),
                            bytes.len()
                        ),
                    );
                    return None;
                }
            };
            images.insert(source.to_string(), Arc::new(rgba));
        }
        let ptr = Arc::as_ptr(images.get(source)?);
        // SAFETY: the map only inserts. The decoded pixels stay allocated until this
        // service drops, which is after `submit` has finished reading them.
        Some(unsafe { &*ptr })
    }
}

impl Services for AndroidServices {
    fn font(&self, family: &str) -> Option<&[u8]> {
        if family != FONT_FAMILY {
            return None;
        }
        self.font_bytes()
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
        log_line(level, message);
    }

    fn set_ime(&self, enabled: bool) {
        self.ime.push(if enabled {
            ImeCommand::Show
        } else {
            ImeCommand::Hide
        });
        self.waker.wake();
    }

    fn set_ime_text(&self, text: &str) {
        self.ime.push(ImeCommand::Text(text.to_owned()));
        self.waker.wake();
    }
}

fn load_asset(assets: &AssetManager, path: &CStr) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut asset = assets.open(path).ok_or_else(|| "open failed".to_string())?;
    match asset.buffer() {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes.to_vec()),
        other => {
            let why = match other {
                Ok(_) => "empty buffer".to_string(),
                Err(error) => error.to_string(),
            };
            let mut bytes = Vec::new();
            asset
                .read_to_end(&mut bytes)
                .map_err(|error| format!("{why}; read {error}"))?;
            if bytes.is_empty() {
                Err(format!("{why}; read empty"))
            } else {
                Ok(bytes)
            }
        }
    }
}

fn asset_image_path(source: &str) -> Option<CString> {
    let name = source.strip_prefix("asset:")?;
    if name.is_empty() {
        return None;
    }
    let mut relative = String::from("images");
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains('\\') {
            return None;
        }
        relative.push('/');
        relative.push_str(part);
    }
    relative.push_str(".png");
    CString::new(relative).ok()
}

pub fn run(app: AndroidApp) -> i32 {
    let assets = Arc::new(app.asset_manager());
    let script_assets = Arc::clone(&assets);
    let clock = FrameClock::new(app.create_waker());
    let ime = Arc::new(ImeMailbox {
        commands: Mutex::new(VecDeque::new()),
    });
    let ime_for_services = Arc::clone(&ime);
    let boot = Boot {
        font_family: FONT_FAMILY.to_string(),
        launch: Launch::Session(Box::new(move |slot| {
            let bytes = decode::read_bytes(&script_assets, SCRIPT_PATH)
                .ok_or_else(|| SessionError::Runtime)?;
            let script = String::from_utf8(bytes).map_err(|_| SessionError::Runtime)?;
            quickjs::open(&script, slot.shaper(), slot.log_hook()).map_err(|error| match error {
                quickjs::Error::NotLinked => SessionError::NotLinked,
                quickjs::Error::Runtime => SessionError::Runtime,
                quickjs::Error::Script(text) => SessionError::Script(text),
            })
        })),
        services: Box::new(AndroidServices {
            assets,
            font: OnceLock::new(),
            images: Mutex::new(HashMap::new()),
            ime: ime_for_services,
            waker: app.create_waker(),
        }),
        vsync: Arc::clone(&clock) as Arc<dyn Vsync>,
        open_surface: Box::new(|engine, target| {
            Ok(Box::new(VulkanSurface::open(engine, target.layer)?))
        }),
    };
    let link = match hostshell::start(boot) {
        Ok(link) => link,
        Err(error) => {
            log_line(LogLevel::Error, &error.to_string());
            return i32::from(error.code());
        }
    };

    let mut window: Option<NativeWindow> = None;
    let mut failed: Option<i32> = None;
    let mut leaving = false;
    let mut ime_text = Snapshot::empty();
    let mut ime_bottom = 0;
    loop {
        app.poll_events(None, |event| {
            apply_ime(&app, &ime, &mut ime_text);
            match event {
                PollEvent::Main(MainEvent::InitWindow { .. }) => {
                    if window.is_some() {
                        link.destroy_surface_and_wait();
                        window = None;
                    }
                    if let Some(next) = app.native_window() {
                        configure_window_buffers(&next);
                        match scene_metrics(&app, &next) {
                            Some(metrics) => {
                                let ptr = next.ptr().as_ptr().cast();
                                window = Some(next);
                                link.post(Command::SurfaceReady(SurfaceTarget {
                                    layer: NativePtr(ptr),
                                    metrics,
                                }));
                            }
                            None => {
                                log_line(LogLevel::Error, "display density was not provided");
                                failed = Some(1);
                            }
                        }
                    }
                }
                PollEvent::Main(MainEvent::WindowResized { .. }) => {
                    if let Some(held) = window.as_ref() {
                        configure_window_buffers(held);
                    }
                    observe_ime(&app, &link, &mut ime_bottom);
                    post_surface_changed(&app, &link, window.as_ref());
                }
                PollEvent::Main(
                    MainEvent::InsetsChanged { .. } | MainEvent::ContentRectChanged { .. },
                ) => {
                    observe_ime(&app, &link, &mut ime_bottom);
                    post_surface_changed(&app, &link, window.as_ref());
                }
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                    link.destroy_surface_and_wait();
                    window = None;
                }
                PollEvent::Main(MainEvent::Pause) => {
                    // Cancel first so a frame requested for a synthetic pointer cancel is not dropped.
                    clock.cancel();
                    link.post(Command::Lifecycle(hostshell::Lifecycle::Pause));
                }
                PollEvent::Main(MainEvent::Resume { .. }) => {
                    link.post(Command::Lifecycle(hostshell::Lifecycle::Resume));
                }
                PollEvent::Main(MainEvent::Destroy) => {
                    link.destroy_surface_and_wait();
                    window = None;
                    leaving = true;
                }
                _ => {}
            }
        });
        if let Some(code) = failed {
            let _ = link.shutdown_and_join();
            return code;
        }
        if leaving {
            return finish(&link);
        }
        pump_input(&app, &link, &mut ime_text, ime_bottom > 0);
        clock.arm_if_needed();
    }
}

fn finish(link: &hostshell::Link) -> i32 {
    match link.shutdown_and_join() {
        Ok(()) => 0,
        Err(error) => {
            log_line(LogLevel::Error, &error.to_string());
            i32::from(error.code())
        }
    }
}

fn pump_input(app: &AndroidApp, link: &hostshell::Link, text: &mut Snapshot, ime_visible: bool) {
    let Ok(mut events) = app.input_events_iter() else {
        return;
    };
    let density = match app.config().density().and_then(density_scale) {
        Some(density) => density,
        None => return,
    };
    loop {
        let read = events.next(|event| match event {
            InputEvent::MotionEvent(motion) => {
                if let Some(pointer) = accepted_pointer(motion, density) {
                    link.post(Command::Pointer(pointer));
                    InputStatus::Handled
                } else {
                    InputStatus::Unhandled
                }
            }
            InputEvent::KeyEvent(key) => match textinput::map_key(
                key.action() == KeyAction::Down,
                key.key_code().into(),
                ime_visible,
            ) {
                Some(KeyOutcome::Dismiss) => {
                    link.post(Command::ImeDismissed);
                    InputStatus::Handled
                }
                Some(KeyOutcome::Edit(edit)) => {
                    if let Some(event) = edit_event(edit) {
                        link.post(Command::Input(event));
                    }
                    InputStatus::Handled
                }
                None => InputStatus::Unhandled,
            },
            InputEvent::TextEvent(state) => {
                let next = snapshot_from(state);
                for edit in textinput::diff(text, &next) {
                    if let Some(event) = edit_event(edit) {
                        link.post(Command::Input(event));
                    }
                }
                *text = next;
                InputStatus::Handled
            }
            InputEvent::TextAction(action) => match action {
                TextInputAction::Done
                | TextInputAction::Go
                | TextInputAction::Send
                | TextInputAction::Search => {
                    link.post(Command::Input(EditEvent::Key(Key::Enter)));
                    InputStatus::Handled
                }
                _ => InputStatus::Unhandled,
            },
            _ => InputStatus::Unhandled,
        });
        if !read {
            break;
        }
    }
}

fn edit_event(edit: Edit) -> Option<EditEvent> {
    Some(match edit {
        Edit::Backspace => EditEvent::Key(Key::Backspace),
        Edit::Enter => EditEvent::Key(Key::Enter),
        Edit::Tab => EditEvent::Key(Key::Tab),
        Edit::Left => EditEvent::Key(Key::Left),
        Edit::Right => EditEvent::Key(Key::Right),
        Edit::Up => EditEvent::Key(Key::Up),
        Edit::Down => EditEvent::Key(Key::Down),
        Edit::Text(text) => EditEvent::Text(text),
        Edit::Composing(text) => EditEvent::Composing(text),
    })
}

fn snapshot_from(state: &TextInputState) -> Snapshot {
    let compose = state.compose_region.and_then(|span| {
        let start = span.start.min(span.end);
        let end = span.start.max(span.end);
        (start != end).then_some((start, end))
    });
    Snapshot {
        text: state.text.clone(),
        compose,
    }
}

fn apply_ime(app: &AndroidApp, mailbox: &ImeMailbox, text: &mut Snapshot) {
    for command in mailbox.drain() {
        match command {
            ImeCommand::Show => {
                app.set_ime_editor_info(
                    InputType::TYPE_CLASS_TEXT,
                    TextInputAction::Done,
                    ImeOptions::IME_FLAG_NO_FULLSCREEN,
                );
                app.show_soft_input(false);
            }
            ImeCommand::Hide => app.hide_soft_input(false),
            ImeCommand::Text(value) => {
                *text = textinput::seed(&value);
                let end = textinput::utf16_len(&text.text);
                app.set_text_input_state(TextInputState {
                    text: text.text.clone(),
                    selection: TextSpan { start: end, end },
                    compose_region: None,
                });
            }
        }
    }
}

fn observe_ime(app: &AndroidApp, link: &hostshell::Link, ime_bottom: &mut i32) {
    let next = java_insets(app, IME)
        .map(|edges| edges.bottom.max(0))
        .unwrap_or(0);
    if *ime_bottom > 0 && next == 0 {
        link.post(Command::ImeDismissed);
    }
    *ime_bottom = next;
}

fn accepted_pointer(
    motion: &android_activity::input::MotionEvent<'_>,
    density: f32,
) -> Option<core::PointerEvent> {
    let phase = motion::phase_for(match motion.action() {
        MotionAction::Down => Action::Down,
        MotionAction::Move => Action::Move,
        MotionAction::Up => Action::Up,
        MotionAction::Cancel => Action::Cancel,
        MotionAction::PointerDown => Action::PointerDown,
        MotionAction::PointerUp => Action::PointerUp,
        _ => Action::Other,
    })?;
    let pointer = motion
        .pointers()
        .find(|pointer| pointer.pointer_index() == 0)?;
    let (x, y) = scene_point(pointer.x(), pointer.y(), density)?;
    Some(core::PointerEvent {
        phase,
        x,
        y,
        // `event_time` is nanoseconds. Velocity only needs deltas on this clock.
        time_ms: motion.event_time() as f64 / 1_000_000.0,
    })
}

fn configure_window_buffers(window: &NativeWindow) {
    let width = window.width();
    let height = window.height();
    if width <= 1 || height <= 1 {
        log_line(
            LogLevel::Error,
            &format!("window buffer is {width}x{height}"),
        );
        return;
    }
    if let Err(error) = window.set_buffers_geometry(
        width,
        height,
        Some(ndk::hardware_buffer_format::HardwareBufferFormat::R8G8B8A8_UNORM),
    ) {
        log_line(
            LogLevel::Error,
            &format!("buffer geometry {width}x{height}: {error}"),
        );
    }
    engine::note_swapchain_extent(width as u32, height as u32);
}

fn post_surface_changed(app: &AndroidApp, link: &hostshell::Link, window: Option<&NativeWindow>) {
    let Some(held) = window else {
        return;
    };
    let Some(metrics) = scene_metrics(app, held) else {
        return;
    };
    link.post(Command::SurfaceChanged(SurfaceTarget {
        layer: NativePtr(held.ptr().as_ptr().cast()),
        metrics,
    }));
}

fn scene_metrics(app: &AndroidApp, window: &NativeWindow) -> Option<Metrics> {
    let density = density_scale(app.config().density()?)?;
    let (width, height) = scene_point(window.width() as f32, window.height() as f32, density)?;
    let (left, top, right, bottom) = system_bar_insets(app).unwrap_or_else(|| {
        let content = app.content_rect();
        content_insets(
            window.width(),
            window.height(),
            content.left,
            content.top,
            content.right,
            content.bottom,
        )
        .unwrap_or((0, 0, 0, 0))
    });
    let (inset_left, inset_top, inset_right, inset_bottom) =
        insets_in_points(left, top, right, bottom, density)?;
    Some(Metrics {
        width,
        height,
        device_pixel_ratio: density,
        inset_left,
        inset_top,
        inset_right,
        inset_bottom,
    })
}

/// Status and navigation bars, in pixels. `content_rect` stays the full buffer
/// when the surface is laid out in screen, so the bars come from GameActivity.
fn system_bar_insets(app: &AndroidApp) -> Option<(i32, i32, i32, i32)> {
    let status = java_insets(app, STATUS_BARS)?;
    let navigation = java_insets(app, NAVIGATION_BARS).unwrap_or(InsetEdges::default());
    let ime = java_insets(app, IME).unwrap_or(InsetEdges::default());
    log_line(
        LogLevel::Info,
        &format!(
            "insets status {} {} {} {} nav {} {} {} {} ime {}",
            status.left,
            status.top,
            status.right,
            status.bottom,
            navigation.left,
            navigation.top,
            navigation.right,
            navigation.bottom,
            ime.bottom
        ),
    );
    Some((
        status.left.max(navigation.left).max(ime.left),
        status.top.max(navigation.top).max(ime.top),
        status.right.max(navigation.right).max(ime.right),
        status.bottom.max(navigation.bottom).max(ime.bottom),
    ))
}

/// `WindowInsets.Type` values. Status bars are `1 << 0`, navigation bars `1 << 1`,
/// IME is `1 << 3`. `targetSdk` 35 keeps the surface full screen, so the keyboard
/// height arrives here instead of shrinking the buffer.
const STATUS_BARS: i32 = 1;
const NAVIGATION_BARS: i32 = 2;
const IME: i32 = 8;

#[derive(Clone, Copy, Default)]
struct InsetEdges {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn java_insets(app: &AndroidApp, kind: i32) -> Option<InsetEdges> {
    let vm_ptr = app.vm_as_ptr();
    let activity_ptr = app.activity_as_ptr();
    if vm_ptr.is_null() || activity_ptr.is_null() {
        return None;
    }
    // SAFETY: both pointers belong to this `AndroidApp` and stay valid until it drops.
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) };
    vm.attach_current_thread(|env| {
        let activity = unsafe { jni::objects::JObject::from_raw(env, activity_ptr.cast()) };
        let value = env.call_method(
            &activity,
            jni::jni_str!("getWindowInsets"),
            jni::jni_sig!("(I)Landroidx/core/graphics/Insets;"),
            &[jni::objects::JValue::Int(kind)],
        )?;
        let insets = value.l()?;
        let edges = if insets.is_null() {
            InsetEdges::default()
        } else {
            InsetEdges {
                left: env
                    .get_field(&insets, jni::jni_str!("left"), jni::jni_sig!("I"))?
                    .i()?,
                top: env
                    .get_field(&insets, jni::jni_str!("top"), jni::jni_sig!("I"))?
                    .i()?,
                right: env
                    .get_field(&insets, jni::jni_str!("right"), jni::jni_sig!("I"))?
                    .i()?,
                bottom: env
                    .get_field(&insets, jni::jni_str!("bottom"), jni::jni_sig!("I"))?
                    .i()?,
            }
        };
        let _ = activity.into_raw();
        Ok::<InsetEdges, jni::errors::Error>(edges)
    })
    .map_err(|error| {
        log_line(LogLevel::Error, &format!("window insets: {error}"));
        error
    })
    .ok()
}

pub(crate) fn log_line(level: LogLevel, message: &str) {
    let priority = match level {
        LogLevel::Error => 6,
        LogLevel::Info => 4,
    };
    let mut bytes = message.replace('\0', " ").into_bytes();
    bytes.push(0);
    // SAFETY: `bytes` is NUL-terminated and lives for this call. The tag is a literal.
    unsafe {
        __android_log_write(priority, c"uifw".as_ptr(), bytes.as_ptr().cast());
    }
}
