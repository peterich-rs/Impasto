//! UI thread shell. `HostActivity` and `HostView` own the window, the surface,
//! and the input connection. This thread only enqueues. The render thread owns
//! the engine. `ANativeWindow` is released after the render thread drops it.

use std::ffi::{CStr, CString, c_char};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use hostshell::{
    Boot, ClipboardHost, Command, DropWait, Launch, LinkingHost, LogLevel, NativePtr, Services,
    SessionError, StorageHost, SurfaceId, SurfaceTarget, Vsync,
};
use jni::objects::{Global, JObject, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};
use ndk::asset::AssetManager;
use ndk::native_window::NativeWindow;
use session::{InputEvent as EditEvent, Key, Metrics};

use crate::decode;
use crate::insets::{insets_in_points, scene_point};
use crate::motion::{self, android_wheel};
use crate::surface::VulkanSurface;
use crate::textinput::{self, Client, Edit, KeyOutcome};
use crate::vsync::FrameClock;

const FONT_FAMILY: &str = "UIFWSans";
const FONT_PATH: &CStr = c"fonts/UIFWSans.ttf";
const SCRIPT_NAME: &str = "counter.js";
const IME_SHOW: i32 = 1;
const IME_UPDATE: i32 = 2;
const IME_HIDE: i32 = 3;

unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
}

struct Bridge {
    vm: jni::JavaVM,
    view: Global<JObject<'static>>,
    assets: Arc<AssetManager>,
    font: OnceLock<Vec<u8>>,
    semantics: Mutex<Option<core::SemanticsUpdate>>,
}

impl Bridge {
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

    fn call(&self, body: impl FnOnce(&mut Env) -> jni::errors::Result<()>) {
        let result = self.vm.attach_current_thread(|env| body(env));
        if let Err(error) = result {
            log_line(LogLevel::Error, &format!("jni: {error}"));
        }
    }

    fn schedule_frame(&self) {
        self.call(|env| {
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("scheduleFrame"),
                jni::jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    fn finish(&self) {
        self.call(|env| {
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("finishHost"),
                jni::jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    fn write_clipboard_text(&self, text: &str) {
        self.call(|env| {
            let text = env.new_string(text)?;
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("setClipboard"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[jni::objects::JValue::Object(&text)],
            )?;
            Ok(())
        });
    }

    fn write_clipboard_now(&self, text: &str) {
        self.call(|env| {
            let text = env.new_string(text)?;
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("writeClipboardNow"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[jni::objects::JValue::Object(&text)],
            )?;
            Ok(())
        });
    }

    fn post_work(&self, work: Box<dyn FnOnce() + Send>) {
        let boxed = Box::into_raw(Box::new(work));
        let mut posted = false;
        self.call(|env| {
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("postNative"),
                jni::jni_sig!("(J)V"),
                &[jni::objects::JValue::Long(boxed as jlong)],
            )?;
            posted = true;
            Ok(())
        });
        if !posted {
            // SAFETY: `boxed` came from `Box::into_raw` above and Java did not take it.
            drop(unsafe { Box::from_raw(boxed) });
        }
    }

    fn launch_view_url(&self, url: &str) -> Result<(), String> {
        self.call(|env| {
            let url = env.new_string(url)?;
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("openUrl"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[jni::objects::JValue::Object(&url)],
            )?;
            Ok(())
        });
        Ok(())
    }

    fn clipboard_text(&self) -> String {
        let mut copied = String::new();
        self.call(|env| {
            let value = env
                .call_method(
                    self.view.as_obj(),
                    jni::jni_str!("readClipboard"),
                    jni::jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if !value.is_null() {
                let string = unsafe { JString::from_raw(env, value.as_raw()) };
                copied = string.try_to_string(env)?;
            }
            Ok(())
        });
        copied
    }

    fn publish_semantics(&self, update: &core::SemanticsUpdate) {
        let nodes = crate::a11y::bridge_nodes(update);
        self.call(|env| {
            let n = nodes.len();
            let ids = jni::objects::JLongArray::new(env, n)?;
            let roles = jni::objects::JIntArray::new(env, n)?;
            let flags = jni::objects::JIntArray::new(env, n)?;
            let actions = jni::objects::JIntArray::new(env, n)?;
            let parents = jni::objects::JLongArray::new(env, n)?;
            let scroll = jni::objects::JFloatArray::new(env, n.saturating_mul(4))?;
            let mut id_buf = vec![0i64; n];
            let mut role_buf = vec![0i32; n];
            let mut flag_buf = vec![0i32; n];
            let mut action_buf = vec![0i32; n];
            let mut parent_buf = vec![0i64; n];
            let mut scroll_buf = vec![0.0f32; n.saturating_mul(4)];
            for (index, node) in nodes.iter().enumerate() {
                id_buf[index] = node.id as i64;
                role_buf[index] = node.role;
                flag_buf[index] = node.flags as i32;
                action_buf[index] = node.actions as i32;
                parent_buf[index] = node.parent.unwrap_or(0) as i64;
                let base = index.saturating_mul(4);
                scroll_buf[base] = node.offset_x;
                scroll_buf[base + 1] = node.offset_y;
                scroll_buf[base + 2] = node.content_w;
                scroll_buf[base + 3] = node.content_h;
            }
            ids.set_region(env, 0, &id_buf)?;
            roles.set_region(env, 0, &role_buf)?;
            flags.set_region(env, 0, &flag_buf)?;
            actions.set_region(env, 0, &action_buf)?;
            parents.set_region(env, 0, &parent_buf)?;
            scroll.set_region(env, 0, &scroll_buf)?;
            let empty = env.new_string("")?;
            let labels = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let values = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let descriptions = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            for (index, node) in nodes.iter().enumerate() {
                let label = env.new_string(&node.label)?;
                labels.set_element(env, index, &label)?;
                let value = env.new_string(&node.value)?;
                values.set_element(env, index, &value)?;
                let description = env.new_string(&node.description)?;
                descriptions.set_element(env, index, &description)?;
            }
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("publishSemantics"),
                jni::jni_sig!(
                    "([J[I[Ljava/lang/String;[Ljava/lang/String;[I[I[F[Ljava/lang/String;[J)V"
                ),
                &[
                    jni::objects::JValue::Object(ids.as_ref()),
                    jni::objects::JValue::Object(roles.as_ref()),
                    jni::objects::JValue::Object(labels.as_ref()),
                    jni::objects::JValue::Object(values.as_ref()),
                    jni::objects::JValue::Object(flags.as_ref()),
                    jni::objects::JValue::Object(actions.as_ref()),
                    jni::objects::JValue::Object(scroll.as_ref()),
                    jni::objects::JValue::Object(descriptions.as_ref()),
                    jni::objects::JValue::Object(parents.as_ref()),
                ],
            )?;
            Ok(())
        });
    }

    fn apply_ime(&self, call: ImeCall) {
        self.call(|env| {
            let text = env.new_string(&call.text)?;
            env.call_method(
                self.view.as_obj(),
                jni::jni_str!("applyIme"),
                jni::jni_sig!("(ILjava/lang/String;IIIIFFFFII)V"),
                &[
                    jni::objects::JValue::Int(call.op),
                    jni::objects::JValue::Object(&text),
                    jni::objects::JValue::Int(call.sel_start),
                    jni::objects::JValue::Int(call.sel_end),
                    jni::objects::JValue::Int(call.comp_start),
                    jni::objects::JValue::Int(call.comp_end),
                    jni::objects::JValue::Float(call.caret_x),
                    jni::objects::JValue::Float(call.caret_y),
                    jni::objects::JValue::Float(call.caret_w),
                    jni::objects::JValue::Float(call.caret_h),
                    jni::objects::JValue::Int(call.input_type),
                    jni::objects::JValue::Int(call.ime_action),
                ],
            )?;
            Ok(())
        });
    }
}

struct ImeCall {
    op: i32,
    text: String,
    sel_start: i32,
    sel_end: i32,
    comp_start: i32,
    comp_end: i32,
    caret_x: f32,
    caret_y: f32,
    caret_w: f32,
    caret_h: f32,
    input_type: i32,
    ime_action: i32,
}

impl ImeCall {
    fn from_state(op: i32, state: &session::ImeState) -> Self {
        let (comp_start, comp_end) = match &state.composing {
            Some(range) => (range.start as i32, range.end as i32),
            None => (-1, -1),
        };
        Self {
            op,
            text: state.text.clone(),
            sel_start: state.selection.start as i32,
            sel_end: state.selection.end as i32,
            comp_start,
            comp_end,
            caret_x: state.caret.x,
            caret_y: state.caret.y,
            caret_w: state.caret.w,
            caret_h: state.caret.h,
            input_type: input_type(state.keyboard),
            ime_action: ime_action(state.action),
        }
    }

    fn hide() -> Self {
        Self {
            op: IME_HIDE,
            text: String::new(),
            sel_start: 0,
            sel_end: 0,
            comp_start: -1,
            comp_end: -1,
            caret_x: 0.0,
            caret_y: 0.0,
            caret_w: 0.0,
            caret_h: 0.0,
            input_type: input_type(session::KeyboardType::Text),
            ime_action: ime_action(session::TextInputAction::Done),
        }
    }
}

impl ClipboardHost for Bridge {
    fn read_clipboard(&self) -> String {
        self.clipboard_text()
    }

    fn write_clipboard(&self, text: &str) {
        self.write_clipboard_now(text);
    }
}

impl LinkingHost for Bridge {
    fn open_url(&self, url: &str) -> Result<(), String> {
        self.launch_view_url(url)
    }
}

impl StorageHost for Bridge {
    fn document_dir(&self) -> Result<String, String> {
        let mut copied = String::new();
        self.call(|env| {
            let value = env
                .call_method(
                    self.view.as_obj(),
                    jni::jni_str!("documentDirectory"),
                    jni::jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()?;
            if !value.is_null() {
                let string = unsafe { JString::from_raw(env, value.as_raw()) };
                copied = string.try_to_string(env)?;
            }
            Ok(())
        });
        if copied.is_empty() {
            Err("module \"FileSystem\" is not linked".to_owned())
        } else {
            Ok(copied)
        }
    }

    fn pick_document(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
}

struct Shared(Arc<Bridge>);

impl Services for Shared {
    fn font(&self, family: &str) -> Option<&[u8]> {
        if family != FONT_FAMILY {
            return None;
        }
        self.0.font_bytes()
    }

    fn image<'a>(&'a self, _source: &str) -> Option<engine::ImagePixels<'a>> {
        None
    }

    fn semantics(&self, update: &core::SemanticsUpdate) {
        *self
            .0
            .semantics
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(update.clone());
        self.0.publish_semantics(update);
    }

    fn log(&self, level: LogLevel, message: &str) {
        log_line(level, message);
    }

    fn request_exit(&self) {
        self.0.finish();
    }

    fn set_clipboard(&self, text: &str) {
        self.0.write_clipboard_text(text);
    }

    fn extra_fonts(&self) -> Vec<(String, Vec<u8>)> {
        const PATHS: &[&str] = &[
            "/system/fonts/NotoSansCJK-Regular.ttc",
            "/system/fonts/NotoSansSC-Regular.otf",
            "/system/fonts/NotoSerifCJK-Regular.ttc",
        ];
        for path in PATHS {
            if let Ok(bytes) = std::fs::read(path)
                && !bytes.is_empty()
            {
                return vec![("UIFWSansCJK".to_string(), bytes)];
            }
        }
        Vec::new()
    }

    fn apply_ime(&self, update: &session::ImeUpdate) {
        let call = match update {
            session::ImeUpdate::Show(state) => ImeCall::from_state(IME_SHOW, state),
            session::ImeUpdate::Update(state) => ImeCall::from_state(IME_UPDATE, state),
            session::ImeUpdate::Hide => ImeCall::hide(),
        };
        self.0.apply_ime(call);
    }
}

struct Viewport {
    width_px: i32,
    height_px: i32,
    density: f32,
    pad_left: i32,
    pad_top: i32,
    pad_right: i32,
    pad_bottom: i32,
    ime_left: i32,
    ime_top: i32,
    ime_right: i32,
    ime_bottom: i32,
}

struct Ui {
    window: Option<NativeWindow>,
    surface_id: Option<SurfaceId>,
    viewport: Option<Viewport>,
    client: Client,
    pending_user: Option<String>,
    ime_client: bool,
    ime_bottom: i32,
}

impl Ui {
    fn new() -> Self {
        Self {
            window: None,
            surface_id: None,
            viewport: None,
            client: Client::new(),
            pending_user: None,
            ime_client: false,
            ime_bottom: 0,
        }
    }
}

struct Host {
    link: hostshell::Link,
    clock: Arc<FrameClock>,
    bridge: Arc<Bridge>,
    ui: Mutex<Ui>,
}

static HOST: Mutex<Option<Arc<Host>>> = Mutex::new(None);

fn host() -> Option<Arc<Host>> {
    HOST.lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(Arc::clone)
}

impl Host {
    fn ui<T>(&self, body: impl FnOnce(&mut Ui) -> T) -> T {
        let mut ui = self.ui.lock().unwrap_or_else(|error| error.into_inner());
        body(&mut ui)
    }

    fn clear_pending(&self) {
        self.ui(|ui| ui.pending_user = None);
    }

    fn post_edits(&self, edits: Vec<Edit>) {
        if edits.is_empty() {
            return;
        }
        self.ui(|ui| {
            ui.pending_user = Some(ui.client.text());
        });
        for edit in edits {
            if let Some(event) = edit_event(edit, false) {
                self.link.post(Command::Input(event));
            }
        }
    }
}

fn edit_event(edit: Edit, shift: bool) -> Option<EditEvent> {
    let press = |key| {
        EditEvent::Key(session::KeyPress {
            key,
            shift,
            modifiers: core::Modifiers::empty(),
        })
    };
    Some(match edit {
        Edit::Backspace => press(Key::Backspace),
        Edit::Delete => press(Key::Delete),
        Edit::Enter => press(Key::Enter),
        Edit::Tab => press(Key::Tab),
        Edit::Escape => press(Key::Escape),
        Edit::Left => press(Key::Left),
        Edit::Right => press(Key::Right),
        Edit::Up => press(Key::Up),
        Edit::Down => press(Key::Down),
        Edit::Home => press(Key::Home),
        Edit::End => press(Key::End),
        Edit::WordLeft => press(Key::WordLeft),
        Edit::WordRight => press(Key::WordRight),
        Edit::DocumentStart => press(Key::DocumentStart),
        Edit::DocumentEnd => press(Key::DocumentEnd),
        Edit::DeleteWord => press(Key::DeleteWord),
        Edit::DeleteWordForward => press(Key::DeleteWordForward),
        Edit::SelectAll => EditEvent::SelectAll,
        Edit::Text(text) => EditEvent::Text(text),
        Edit::DeleteRange { start, end } => EditEvent::Edit(core::Edit::DeleteRange { start, end }),
        Edit::Insert(text) => EditEvent::Edit(core::Edit::Insert(text)),
        Edit::Composing(text) => EditEvent::Composing(text),
    })
}

fn input_type(keyboard: session::KeyboardType) -> i32 {
    const CLASS_TEXT: i32 = 1;
    const CLASS_NUMBER: i32 = 2;
    const CLASS_PHONE: i32 = 3;
    const VARIATION_URI: i32 = 0x10;
    const VARIATION_EMAIL: i32 = 0x20;
    const VARIATION_PERSON_NAME: i32 = 0x60;
    const VARIATION_PASSWORD: i32 = 0x80;
    const FLAG_CAP_WORDS: i32 = 0x2000;
    const FLAG_AUTO_CORRECT: i32 = 0x8000;
    const FLAG_MULTI_LINE: i32 = 0x20000;
    const FLAG_NO_SUGGESTIONS: i32 = 0x80000;
    match keyboard {
        session::KeyboardType::Text => CLASS_TEXT | FLAG_AUTO_CORRECT,
        session::KeyboardType::Multiline => CLASS_TEXT | FLAG_MULTI_LINE | FLAG_AUTO_CORRECT,
        session::KeyboardType::Number => CLASS_NUMBER,
        session::KeyboardType::Phone => CLASS_PHONE,
        session::KeyboardType::Email => CLASS_TEXT | VARIATION_EMAIL,
        session::KeyboardType::Url => CLASS_TEXT | VARIATION_URI,
        session::KeyboardType::Name => CLASS_TEXT | VARIATION_PERSON_NAME | FLAG_CAP_WORDS,
        session::KeyboardType::Password => CLASS_TEXT | VARIATION_PASSWORD | FLAG_NO_SUGGESTIONS,
    }
}

fn ime_action(action: session::TextInputAction) -> i32 {
    match action {
        session::TextInputAction::None | session::TextInputAction::Newline => 1,
        session::TextInputAction::Go => 2,
        session::TextInputAction::Search => 3,
        session::TextInputAction::Send => 4,
        session::TextInputAction::Next => 5,
        session::TextInputAction::Done => 6,
        session::TextInputAction::Previous => 7,
    }
}

struct Attach {
    view: Global<JObject<'static>>,
    assets: Arc<AssetManager>,
    script: Option<String>,
    launch_url: Option<String>,
    settings: session::PlatformSettings,
}

fn attach(env: &mut Env, request: Attach) -> Result<(), String> {
    let bridge = Arc::new(Bridge {
        vm: env.get_java_vm().map_err(|error| error.to_string())?,
        view: request.view,
        assets: Arc::clone(&request.assets),
        font: OnceLock::new(),
        semantics: Mutex::new(None),
    });
    let bridge_for_clock = Arc::clone(&bridge);
    let clock = FrameClock::new(Arc::new(move || bridge_for_clock.schedule_frame()));
    let script_assets = Arc::clone(&request.assets);
    let requested = request.script;
    let launch_url = session::shared(None);
    let main_thread = {
        let bridge = Arc::clone(&bridge);
        Arc::new(move |work: Box<dyn FnOnce() + Send>| {
            bridge.post_work(work);
        }) as hostshell::MainThreadPoster
    };
    let boot = Boot {
        font_family: FONT_FAMILY.to_string(),
        launch: Launch::Session(Box::new(move |slot, io, host| {
            let (name, script) = load_script(&script_assets, requested.as_deref())?;
            quickjs::open_with_io(
                &script,
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
        })),
        services: Box::new(Shared(Arc::clone(&bridge))),
        vsync: Arc::clone(&clock) as Arc<dyn Vsync>,
        decoder: Arc::new(AssetDecoder {
            assets: request.assets,
        }),
        modules: Vec::new(),
        main_thread: Some(main_thread),
        launch_url: Arc::clone(&launch_url),
        http: Some(hostshell::http_client()?),
        open_surface: Box::new(|engine, target| {
            Ok(Box::new(VulkanSurface::open(engine, target.layer)?))
        }),
        dump_list: false,
    }
    .with_module(hostshell::clipboard(Arc::clone(&bridge) as _))
    .with_module(hostshell::linking(Arc::clone(&bridge) as _, launch_url))
    .with_module(hostshell::file_system(Arc::clone(&bridge) as _))
    .with_module(hostshell::async_storage(Arc::clone(&bridge) as _));
    let link = hostshell::start(boot).map_err(|error| error.to_string())?;
    link.post(Command::Settings(request.settings));
    if let Some(url) = request.launch_url {
        link.post(Command::LaunchUrl(url));
    }
    *HOST.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::new(Host {
        link,
        clock,
        bridge,
        ui: Mutex::new(Ui::new()),
    }));
    Ok(())
}

fn detach() {
    let Some(host) = HOST
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
    else {
        return;
    };
    destroy_surface(&host);
    if let Err(error) = host.link.shutdown_and_join() {
        log_line(LogLevel::Error, &error.to_string());
    }
}

fn destroy_surface(host: &Host) {
    let (window, id) = host.ui(|ui| (ui.window.take(), ui.surface_id.take()));
    let (Some(window), Some(id)) = (window, id) else {
        return;
    };
    host.link.begin_destroy(id);
    release_when_dropped(host.link.drop_wait(), id, window);
}

/// Leaks the window if the drop thread never starts. After the ack, [`Self::release`] drops it.
struct HeldWindow(Option<NativeWindow>);

impl HeldWindow {
    fn release(&mut self) {
        drop(self.0.take());
    }
}

impl Drop for HeldWindow {
    fn drop(&mut self) {
        if let Some(window) = self.0.take() {
            std::mem::forget(window);
        }
    }
}

fn release_when_dropped(wait: DropWait, id: SurfaceId, window: NativeWindow) {
    let mut held = HeldWindow(Some(window));
    let spawned = thread::Builder::new()
        .name("uifw-surface-drop".into())
        .spawn(move || {
            wait.wait(id);
            held.release();
        });
    if spawned.is_err() {
        log_line(
            LogLevel::Error,
            "surface drop thread did not start; keeping the window",
        );
    }
}

fn open_surface(host: &Host, window: NativeWindow) {
    configure_window_buffers(&window);
    let previous = host.ui(|ui| (ui.window.take(), ui.surface_id.take()));
    if let (Some(window), Some(id)) = previous {
        host.link.begin_destroy(id);
        release_when_dropped(host.link.drop_wait(), id, window);
    }
    host.ui(|ui| ui.window = Some(window));
    post_ready(host);
}

/// Posts `SurfaceReady` once the window and the viewport both exist.
fn post_ready(host: &Host) -> bool {
    let ready = host.ui(|ui| {
        if ui.surface_id.is_some() {
            return None;
        }
        let window = ui.window.as_ref()?;
        let viewport = ui.viewport.as_ref()?;
        let metrics = metrics_of(&Viewport {
            width_px: window.width(),
            height_px: window.height(),
            density: viewport.density,
            pad_left: viewport.pad_left,
            pad_top: viewport.pad_top,
            pad_right: viewport.pad_right,
            pad_bottom: viewport.pad_bottom,
            ime_left: viewport.ime_left,
            ime_top: viewport.ime_top,
            ime_right: viewport.ime_right,
            ime_bottom: viewport.ime_bottom,
        })?;
        let id = SurfaceId::fresh();
        let ptr = window.ptr().as_ptr().cast();
        ui.surface_id = Some(id);
        ui.ime_bottom = viewport.ime_bottom;
        Some(SurfaceTarget {
            id,
            layer: NativePtr(ptr),
            metrics,
        })
    });
    if let Some(target) = ready {
        host.link.post(Command::SurfaceReady(target));
        true
    } else {
        false
    }
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

fn metrics_of(viewport: &Viewport) -> Option<Metrics> {
    let (width, height) = scene_point(
        viewport.width_px as f32,
        viewport.height_px as f32,
        viewport.density,
    )?;
    let (pad_left, pad_top, pad_right, pad_bottom) = insets_in_points(
        viewport.pad_left,
        viewport.pad_top,
        viewport.pad_right,
        viewport.pad_bottom,
        viewport.density,
    )?;
    let (ime_left, ime_top, ime_right, ime_bottom) = insets_in_points(
        viewport.ime_left,
        viewport.ime_top,
        viewport.ime_right,
        viewport.ime_bottom,
        viewport.density,
    )?;
    Some(Metrics::from_insets(
        width,
        height,
        viewport.density,
        session::Insets {
            left: pad_left,
            top: pad_top,
            right: pad_right,
            bottom: pad_bottom,
        },
        session::Insets {
            left: ime_left,
            top: ime_top,
            right: ime_right,
            bottom: ime_bottom,
        },
    ))
}

fn publish_metrics(host: &Host) {
    if post_ready(host) {
        return;
    }
    let (id, metrics, dismiss) = host.ui(|ui| {
        let Some(window) = ui.window.as_ref() else {
            return (None, None, false);
        };
        let Some(viewport) = ui.viewport.as_ref() else {
            return (None, None, false);
        };
        let sized = Viewport {
            width_px: window.width(),
            height_px: window.height(),
            density: viewport.density,
            pad_left: viewport.pad_left,
            pad_top: viewport.pad_top,
            pad_right: viewport.pad_right,
            pad_bottom: viewport.pad_bottom,
            ime_left: viewport.ime_left,
            ime_top: viewport.ime_top,
            ime_right: viewport.ime_right,
            ime_bottom: viewport.ime_bottom,
        };
        let dismiss = ui.ime_bottom > 0 && sized.ime_bottom == 0;
        ui.ime_bottom = sized.ime_bottom;
        (ui.surface_id, metrics_of(&sized), dismiss)
    });
    if dismiss {
        host.link.post(Command::ImeDismissed);
    }
    if let (Some(id), Some(metrics)) = (id, metrics) {
        host.link.post(Command::SurfaceChanged { id, metrics });
    }
}

fn load_script(
    assets: &AssetManager,
    requested: Option<&str>,
) -> Result<(String, String), SessionError> {
    let mut names = Vec::new();
    if let Some(path) = requested.and_then(motion::script_asset_path) {
        names.push(path);
    }
    names.push("app.js".to_string());
    names.push(SCRIPT_NAME.to_string());
    for name in names {
        let Ok(path) = CString::new(name.clone()) else {
            continue;
        };
        let Some(bytes) = decode::read_bytes(assets, &path) else {
            continue;
        };
        let script = String::from_utf8(bytes).map_err(|_| SessionError::Runtime)?;
        return Ok((name, script));
    }
    Err(SessionError::Runtime)
}

fn asset_image_path(source: &str, scale_bucket: u8) -> Vec<CString> {
    let scale = f32::from(scale_bucket.max(1));
    crate::assets::asset_image_candidates(source, scale)
        .into_iter()
        .filter_map(|path| CString::new(path).ok())
        .collect()
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

struct AssetDecoder {
    assets: Arc<AssetManager>,
}

impl hostshell::ImageDecoder for AssetDecoder {
    fn decode(&self, source: &str) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        self.decode_at(source, 1)
    }

    fn decode_at(
        &self,
        source: &str,
        scale_bucket: u8,
    ) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        let paths = asset_image_path(source, scale_bucket);
        if paths.is_empty() {
            return Err(hostshell::ImageError::Missing);
        }
        for path in &paths {
            let bytes = match load_asset(&self.assets, path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            return match decode::decode_rgba(&bytes) {
                Ok(rgba) => Ok(image_handle(source, rgba.width, rgba.height, rgba.pixels)),
                Err(message) => Err(hostshell::ImageError::Failed(message)),
            };
        }
        Err(hostshell::ImageError::Missing)
    }

    fn decode_bytes(
        &self,
        source: &str,
        bytes: &[u8],
    ) -> Result<hostshell::ImageHandle, hostshell::ImageError> {
        match decode::decode_rgba(bytes) {
            Ok(rgba) => Ok(image_handle(source, rgba.width, rgba.height, rgba.pixels)),
            Err(message) => Err(hostshell::ImageError::Failed(message)),
        }
    }
}

fn image_handle(source: &str, width: u32, height: u32, pixels: Vec<u8>) -> hostshell::ImageHandle {
    hostshell::ImageHandle {
        source: Arc::from(source),
        generation: 0,
        size: [width, height],
        pixels: Arc::from(pixels),
    }
}

fn settings_from(
    locale: String,
    text_scale: f32,
    dark: bool,
    reduce_motion: bool,
    bold_text: bool,
    high_contrast: bool,
) -> session::PlatformSettings {
    session::PlatformSettings {
        locale,
        text_scale: if text_scale.is_finite() && text_scale > 0.0 {
            text_scale
        } else {
            1.0
        },
        brightness: if dark {
            session::Brightness::Dark
        } else {
            session::Brightness::Light
        },
        reduce_motion,
        bold_text,
        high_contrast,
    }
}

fn java_string(env: &mut Env, value: &JString) -> Result<Option<String>, jni::errors::Error> {
    if value.is_null() {
        Ok(None)
    } else {
        Ok(Some(value.try_to_string(env)?))
    }
}

fn asset_manager(env: &mut Env, assets: &JObject<'_>) -> Result<Arc<AssetManager>, String> {
    let raw = unsafe { ndk_sys::AAssetManager_fromJava(env.get_raw().cast(), assets.as_raw()) };
    let Some(ptr) = NonNull::new(raw) else {
        return Err("asset manager was null".to_string());
    };
    Ok(Arc::new(unsafe { AssetManager::from_ptr(ptr) }))
}

fn flag(value: jboolean) -> bool {
    value
}

pub(crate) fn log_line(level: LogLevel, message: &str) {
    let priority = match level {
        LogLevel::Error => 6,
        LogLevel::Info => 4,
    };
    let mut bytes = message.replace('\0', " ").into_bytes();
    bytes.push(0);
    unsafe {
        __android_log_write(priority, c"uifw".as_ptr(), bytes.as_ptr().cast());
    }
}

fn with_env<'local, T: Default>(
    env: &mut EnvUnowned<'local>,
    body: impl FnOnce(&mut Env<'local>) -> jni::errors::Result<T>,
) -> T {
    env.with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeAttach<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    _activity: JObject<'local>,
    view: JObject<'local>,
    assets: JObject<'local>,
    script: JString<'local>,
    launch_url: JString<'local>,
    locale: JString<'local>,
    text_scale: f32,
    dark: jboolean,
    reduce_motion: jboolean,
    bold_text: jboolean,
    high_contrast: jboolean,
) -> jint {
    with_env(&mut env, |env| {
        let script = java_string(env, &script)?;
        let launch_url = crate::launch::from_intent(java_string(env, &launch_url)?.as_deref());
        let locale = java_string(env, &locale)?.unwrap_or_default();
        let assets = match asset_manager(env, &assets) {
            Ok(assets) => assets,
            Err(message) => {
                log_line(LogLevel::Error, &message);
                return Ok(1);
            }
        };
        let view = env.new_global_ref(&view)?;
        let request = Attach {
            view,
            assets,
            script,
            launch_url,
            settings: settings_from(
                locale,
                text_scale,
                flag(dark),
                flag(reduce_motion),
                flag(bold_text),
                flag(high_contrast),
            ),
        };
        match attach(env, request) {
            Ok(()) => Ok(0),
            Err(message) => {
                log_line(LogLevel::Error, &message);
                Ok(1)
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeDetach(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    detach();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeResume(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.link
        .post(Command::Lifecycle(hostshell::Lifecycle::Resumed));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeInactive(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.link
        .post(Command::Lifecycle(hostshell::Lifecycle::Inactive));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeHidden(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.link
        .post(Command::Lifecycle(hostshell::Lifecycle::Hidden));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeTrimMemory(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.link.post(Command::MemoryWarning);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeFocus(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    focused: jboolean,
) {
    let Some(host) = host() else {
        return;
    };
    host.link.post(Command::WindowFocus(flag(focused)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeBack(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.clear_pending();
    let visible = host.ui(|ui| ui.ime_client || ui.ime_bottom > 0);
    if visible {
        host.link.post(Command::ImeDismissed);
    } else {
        host.link.post(Command::Input(EditEvent::Back));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostActivity_nativeSettings<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    locale: JString<'local>,
    text_scale: f32,
    dark: jboolean,
    reduce_motion: jboolean,
    bold_text: jboolean,
    high_contrast: jboolean,
) {
    with_env(&mut env, |env| {
        let Some(host) = host() else {
            return Ok(());
        };
        let locale = java_string(env, &locale)?.unwrap_or_default();
        host.link.post(Command::Settings(settings_from(
            locale,
            text_scale,
            flag(dark),
            flag(reduce_motion),
            flag(bold_text),
            flag(high_contrast),
        )));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostSurfaceView_nativeSurfaceCreated<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    surface: JObject<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host() else {
            return Ok(());
        };
        let raw =
            unsafe { ndk_sys::ANativeWindow_fromSurface(env.get_raw().cast(), surface.as_raw()) };
        let Some(ptr) = NonNull::new(raw) else {
            log_line(LogLevel::Error, "surface produced no native window");
            return Ok(());
        };
        let window = unsafe { NativeWindow::from_ptr(ptr) };
        open_surface(&host, window);
        publish_metrics(&host);
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostSurfaceView_nativeSurfaceChanged(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    _width: jint,
    _height: jint,
) {
    let Some(host) = host() else {
        return;
    };
    host.ui(|ui| {
        if let Some(window) = ui.window.as_ref() {
            configure_window_buffers(window);
        }
    });
    publish_metrics(&host);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostSurfaceView_nativeSurfaceDestroyed(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    destroy_surface(&host);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostView_nativeMetrics(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    width: jint,
    height: jint,
    density: f32,
    pad_left: jint,
    pad_top: jint,
    pad_right: jint,
    pad_bottom: jint,
    ime_left: jint,
    ime_top: jint,
    ime_right: jint,
    ime_bottom: jint,
) {
    let Some(host) = host() else {
        return;
    };
    host.ui(|ui| {
        ui.viewport = Some(Viewport {
            width_px: width,
            height_px: height,
            density,
            pad_left,
            pad_top,
            pad_right,
            pad_bottom,
            ime_left,
            ime_top,
            ime_right,
            ime_bottom,
        });
    });
    publish_metrics(&host);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostView_nativePointer(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    phase: jint,
    pointer_id: jint,
    x: f32,
    y: f32,
    time_ms: jlong,
    kind: jint,
    button: jint,
    buttons: jint,
    modifiers: jint,
    wheel_x: f32,
    wheel_y: f32,
    density: f32,
) {
    let Some(host) = host() else {
        return;
    };
    let Some((x, y)) = scene_point(x, y, density) else {
        return;
    };
    let kind = match kind {
        1 => core::PointerKind::Mouse,
        2 => core::PointerKind::Pen,
        _ => core::PointerKind::Touch,
    };
    let button = match button {
        1 => core::Button::Primary,
        2 => core::Button::Secondary,
        3 => core::Button::Auxiliary,
        _ => core::Button::None,
    };
    if phase == 5 {
        let Some((dx, dy)) = android_wheel(wheel_x, wheel_y) else {
            return;
        };
        host.link.post(Command::Pointer(core::PointerEvent::wheel(
            x,
            y,
            time_ms as f64,
            dx,
            dy,
        )));
        return;
    }
    let phase = match phase {
        0 => core::Phase::Down,
        1 | 4 => core::Phase::Move,
        2 => core::Phase::Up,
        3 => core::Phase::Cancel,
        _ => return,
    };
    let button = if phase == core::Phase::Cancel {
        core::Button::None
    } else if kind == core::PointerKind::Touch && button == core::Button::None {
        core::Button::Primary
    } else {
        button
    };
    host.link.post(Command::Pointer(core::PointerEvent {
        pointer: core::PointerId(u64::from(pointer_id as u32)),
        kind,
        phase,
        x,
        y,
        time_ms: time_ms as f64,
        button,
        buttons: core::Buttons::from_bits(buttons as u8),
        modifiers: core::Modifiers::from_bits(modifiers as u8),
        dx: 0.0,
        dy: 0.0,
        zoom: 0.0,
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostView_nativeZoom(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    scale: f32,
    x: f32,
    y: f32,
    time_ms: jlong,
    density: f32,
) {
    let Some(host) = host() else {
        return;
    };
    let Some((x, y)) = scene_point(x, y, density) else {
        return;
    };
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    host.link.post(Command::Pointer(core::PointerEvent::zoom(
        x,
        y,
        time_ms as f64,
        scale,
    )));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostView_nativeKey(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    action: jint,
    key_code: jint,
    meta: jint,
    unicode: jint,
) {
    let Some(host) = host() else {
        return;
    };
    let down = action == 0;
    let shift = meta & 1 != 0;
    let ctrl = meta & 2 != 0;
    let ime = host.ui(|ui| ui.ime_client);
    if down {
        let code = key_code as u32;
        if code == textinput::KEY_COPY {
            host.clear_pending();
            host.link.post(Command::Input(EditEvent::Copy));
            return;
        }
        if code == textinput::KEY_CUT {
            host.clear_pending();
            host.link.post(Command::Input(EditEvent::Cut));
            return;
        }
        if code == textinput::KEY_PASTE {
            host.clear_pending();
            let text = host.bridge.clipboard_text();
            host.link.post(Command::Input(EditEvent::Paste {
                plain: text,
                html: None,
            }));
            return;
        }
        if !ime && let Some(text) = char::from_u32(unicode as u32).filter(|ch| !ch.is_control()) {
            let edits = host.ui(|ui| ui.client.commit(&text.to_string()));
            host.post_edits(edits);
            return;
        }
    }
    match textinput::map_key(down, key_code as u32, ime, shift, ctrl) {
        Some(KeyOutcome::Dismiss) => {
            host.clear_pending();
            host.link.post(Command::ImeDismissed);
        }
        Some(KeyOutcome::Back) => {
            host.clear_pending();
            host.link.post(Command::Input(EditEvent::Back));
        }
        Some(KeyOutcome::Edit { edit, shift }) => {
            host.clear_pending();
            if let Some(event) = edit_event(edit, shift) {
                host.link.post(Command::Input(event));
            }
        }
        None => {}
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_TextInput_nativeSync<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    text: JString<'local>,
    sel_start: jint,
    sel_end: jint,
    comp_start: jint,
    comp_end: jint,
) {
    with_env(&mut env, |env| {
        let Some(host) = host() else {
            return Ok(());
        };
        let text = java_string(env, &text)?.unwrap_or_default();
        let start = u32::try_from(sel_start).unwrap_or(0);
        let end = u32::try_from(sel_end).unwrap_or(start);
        let compose = if comp_start >= 0 && comp_end > comp_start {
            Some((comp_start as u32)..(comp_end as u32))
        } else {
            None
        };
        host.ui(|ui| {
            ui.client.apply_state(&text, start..end, compose);
            ui.pending_user = Some(text.clone());
            ui.ime_client = true;
        });
        host.link.post(Command::Input(EditEvent::Replace {
            text,
            start,
            end,
            compose_start: comp_start,
            compose_end: comp_end,
        }));
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_TextInput_nativeAction(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) {
    let Some(host) = host() else {
        return;
    };
    host.clear_pending();
    host.link
        .post(Command::Input(EditEvent::Key(session::KeyPress::new(
            Key::Enter,
        ))));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_TextInput_nativeContext(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    kind: jint,
) {
    let Some(host) = host() else {
        return;
    };
    host.clear_pending();
    match kind {
        1 => host.link.post(Command::Input(EditEvent::Copy)),
        2 => host.link.post(Command::Input(EditEvent::Cut)),
        3 => {
            let text = host.bridge.clipboard_text();
            host.link.post(Command::Input(EditEvent::Paste {
                plain: text,
                html: None,
            }));
        }
        4 => host.link.post(Command::Input(EditEvent::SelectAll)),
        _ => {}
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_TextInput_nativeAccept<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    op: jint,
    text: JString<'local>,
    sel_start: jint,
    sel_end: jint,
    comp_start: jint,
    comp_end: jint,
) -> jboolean {
    with_env(&mut env, |env| {
        let Some(host) = host() else {
            return Ok(false);
        };
        let text = java_string(env, &text)?.unwrap_or_default();
        let accept = host.ui(|ui| {
            if op == IME_HIDE {
                ui.client.apply_state("", 0..0, None);
                ui.pending_user = None;
                ui.ime_client = false;
                return true;
            }
            if ui
                .pending_user
                .as_deref()
                .is_some_and(|expected| expected != text)
            {
                return false;
            }
            ui.pending_user = None;
            ui.ime_client = true;
            let compose = if comp_start < 0 || comp_end < comp_start {
                None
            } else {
                Some((comp_start as u32)..(comp_end as u32))
            };
            let start = u32::try_from(sel_start).unwrap_or(0);
            let end = u32::try_from(sel_end).unwrap_or(start);
            ui.client.apply_state(&text, start..end, compose);
            true
        });
        Ok(accept)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_HostView_nativeRunWork(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    work: jlong,
) {
    if work == 0 {
        return;
    }
    // SAFETY: `work` is the pointer `post_work` gave Java, and this is the only take.
    let work = unsafe { Box::from_raw(work as *mut Box<dyn FnOnce() + Send>) };
    work();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_VsyncWaiter_nativeOnVsync(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    frame_time_nanos: jlong,
    target_nanos: jlong,
    interval_nanos: jlong,
) {
    let Some(host) = host() else {
        return;
    };
    host.clock
        .on_vsync(frame_time_nanos, target_nanos, interval_nanos);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_AccessBridge_nativeActivate(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    id: jlong,
) {
    let Some(host) = host() else {
        return;
    };
    host.link
        .post(Command::SemanticsAction(core::SemanticsAction::Activate(
            core::SemanticsId(id as u64),
        )));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_AccessBridge_nativeScroll(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    id: jlong,
    forward: jboolean,
) {
    let Some(host) = host() else {
        return;
    };
    let id = core::SemanticsId(id as u64);
    let (dx, dy) = {
        let update = host
            .bridge
            .semantics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(update) = update.as_ref() else {
            return;
        };
        crate::a11y::scroll_action(update, id, forward)
    };
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    host.link
        .post(Command::SemanticsAction(core::SemanticsAction::Scroll {
            id,
            dx,
            dy,
        }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_uifw_host_AccessBridge_nativeSetText<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    id: jlong,
    text: JString<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host() else {
            return Ok(());
        };
        let text = java_string(env, &text)?.unwrap_or_default();
        host.link
            .post(Command::SemanticsAction(core::SemanticsAction::SetText {
                id: core::SemanticsId(id as u64),
                text,
            }));
        Ok(())
    });
}

#[cfg(test)]
mod ime_map {
    use super::{ime_action, input_type};
    use session::{KeyboardType, TextInputAction};

    #[test]
    fn action_ordinals_match_android_ime_action() {
        assert_eq!(ime_action(TextInputAction::None), 1);
        assert_eq!(ime_action(TextInputAction::Newline), 1);
        assert_eq!(ime_action(TextInputAction::Go), 2);
        assert_eq!(ime_action(TextInputAction::Search), 3);
        assert_eq!(ime_action(TextInputAction::Send), 4);
        assert_eq!(ime_action(TextInputAction::Next), 5);
        assert_eq!(ime_action(TextInputAction::Done), 6);
        assert_eq!(ime_action(TextInputAction::Previous), 7);
    }

    #[test]
    fn keyboard_ordinals_use_distinct_input_classes() {
        assert_ne!(
            input_type(KeyboardType::Text),
            input_type(KeyboardType::Number)
        );
        assert_ne!(
            input_type(KeyboardType::Phone),
            input_type(KeyboardType::Email)
        );
        assert_ne!(
            input_type(KeyboardType::Password),
            input_type(KeyboardType::Text)
        );
        assert_ne!(
            input_type(KeyboardType::Multiline),
            input_type(KeyboardType::Text)
        );
    }
}
