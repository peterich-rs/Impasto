//! Android JNI adapter. `ImpastoActivity` and `ImpastoView` own the window, the surface,
//! and the input connection. The render thread owns the engine. The Android
//! surface-destroy callback waits for that thread to stop using `ANativeWindow`.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use embedder::{
    Boot, BootError, Command, FrameRateHint, Launch, NativePtr, Services, SessionError, SurfaceId,
    SurfaceTarget, Vsync,
};
use engine::session::{InputEvent as EditEvent, Key, Metrics};
use jni::Env;
use jni::objects::{Global, JObject, JString};
use jni::sys::jlong;
use ndk::asset::AssetManager;
use ndk::native_window::NativeWindow;

use crate::decode;
use crate::insets::{insets_in_points, scene_point};
use crate::motion;
use crate::surface::Renderer;
use crate::textinput::{Client, Edit};
use crate::vsync::FrameClock;

const FONT_FAMILY: &str = "Sans";
const FONT_PATH: &CStr = c"fonts/Sans.ttf";
const IME_SHOW: i32 = 1;
const IME_UPDATE: i32 = 2;
const IME_HIDE: i32 = 3;

pub(crate) mod entrypoints;
mod hardware_frame;

struct Bridge {
    vm: jni::JavaVM,
    view: Mutex<Option<Arc<Global<JObject<'static>>>>>,
    assets: Arc<AssetManager>,
    document_dir: Option<String>,
    font: OnceLock<Vec<u8>>,
    semantics: Mutex<Vec<embedder::ProjectedNode>>,
    semantics_revision: AtomicI64,
    native_channel: impasto::os::NativeChannel,
    native_events: impasto::os::NativeEventChannel,
    restoration: embedder::Restoration,
    window_session: embedder::WindowSession,
    back_enabled: AtomicBool,
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

    fn current_view(&self) -> Option<Arc<Global<JObject<'static>>>> {
        self.view
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn call(&self, body: impl FnOnce(&mut Env, &JObject<'static>) -> jni::errors::Result<()>) {
        let Some(view) = self.current_view() else {
            return;
        };
        let result = self
            .vm
            .attach_current_thread(|env| body(env, view.as_obj()));
        if let Err(error) = result {
            tracing::error!(target: "impasto::shell", %error, "Android JNI callback failed");
        }
    }

    fn schedule_frame(&self, order: u64, request: Option<(embedder::FrameToken, FrameRateHint)>) {
        let (token, hint) = request.unwrap_or((embedder::FrameToken(0), FrameRateHint::Display));
        let preferred_rate = match hint {
            FrameRateHint::Display => 0,
            FrameRateHint::Hz60 => 60,
            FrameRateHint::Hz30 => 30,
        };
        self.call(|env, view| {
            env.call_method(
                view,
                jni::jni_str!("scheduleFrame"),
                jni::jni_sig!("(JJI)V"),
                &[
                    jni::objects::JValue::Long(order as jlong),
                    jni::objects::JValue::Long(token.0 as jlong),
                    jni::objects::JValue::Int(preferred_rate),
                ],
            )?;
            Ok(())
        });
    }

    fn session_started(&self, result: Result<(), BootError>) {
        if let Err(BootError::Cancelled) = result {
            return;
        }
        let status = match result {
            Ok(()) => 0,
            Err(error) => {
                entrypoints::early_log(&error.to_string());
                i32::from(error.code())
            }
        };
        self.call(|env, view| {
            env.call_method(
                view,
                jni::jni_str!("onSessionStarted"),
                jni::jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(status)],
            )?;
            Ok(())
        });
    }

    fn first_frame(&self, id: SurfaceId) {
        self.call(|env, view| {
            env.call_method(
                view,
                jni::jni_str!("onFirstFrameSubmitted"),
                jni::jni_sig!("(J)V"),
                &[jni::objects::JValue::Long(id.0 as jlong)],
            )?;
            Ok(())
        });
    }

    fn finish(&self) {
        self.call(|env, view| {
            env.call_method(view, jni::jni_str!("finishHost"), jni::jni_sig!("()V"), &[])?;
            Ok(())
        });
    }

    fn clipboard_text(&self) -> String {
        let mut copied = String::new();
        self.call(|env, view| {
            let value = env
                .call_method(
                    view,
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

    fn update_result(&self, revision: &str, result: Result<(), &str>) {
        let Some(files) = &self.document_dir else {
            return;
        };
        let directory = std::path::Path::new(files).join("impasto-debug");
        let result = match result {
            Ok(()) => "ok".to_owned(),
            Err(message) => format!("error\n{message}"),
        };
        let status = (|| {
            let temporary = directory.join(format!("v1-{revision}.tmp"));
            std::fs::write(&temporary, result)?;
            std::fs::rename(temporary, directory.join(format!("v1-{revision}.result")))
        })();
        if let Err(error) = status {
            tracing::error!(target: "impasto::script", %error, "could not publish debug update result");
        }
    }

    fn place_platform_views(
        &self,
        surface: SurfaceId,
        placements: &[engine::tree::PlatformViewPlacement],
    ) {
        self.call(|env, view| {
            use jni::objects::{JFloatArray, JLongArray, JObjectArray, JValue};
            let placements: Vec<_> = placements
                .iter()
                .filter(|slot| slot.supports_rectangular_overlay())
                .collect();
            let count = placements.len();
            let ids = JLongArray::new(env, count)?;
            let id_values: Vec<jlong> = placements
                .iter()
                .map(|slot| slot.node.to_raw() as jlong)
                .collect();
            ids.set_region(env, 0, &id_values)?;
            let empty = env.new_string("")?;
            let kinds = JObjectArray::<JString>::new(env, count, &empty)?;
            let geometry = JFloatArray::new(env, count * 8)?;
            let mut rects = Vec::with_capacity(count * 8);
            for (index, slot) in placements.iter().enumerate() {
                let kind = env.new_string(slot.kind.as_ref())?;
                kinds.set_element(env, index, &kind)?;
                for rect in [slot.rect, slot.clip] {
                    rects.extend([rect.x, rect.y, rect.w, rect.h]);
                }
            }
            geometry.set_region(env, 0, &rects)?;
            env.call_method(
                view,
                jni::jni_str!("placePlatformViews"),
                jni::jni_sig!("(J[J[Ljava/lang/String;[F)V"),
                &[
                    JValue::Long(surface.0 as jlong),
                    JValue::Object(ids.as_ref()),
                    JValue::Object(kinds.as_ref()),
                    JValue::Object(geometry.as_ref()),
                ],
            )?;
            Ok(())
        });
    }

    fn publish_semantics(&self, nodes: &[embedder::ProjectedNode], reset: bool) {
        let revision = self.semantics_revision.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::trace!(target: "impasto::semantics", nodes = nodes.len(), revision, reset,
            strings = nodes.len() * 3 + nodes.iter().filter(|node| node.autofill.is_some()).count() * 2,
            "publishing Android semantics");
        let projected = nodes;
        let nodes = crate::a11y::bridge_nodes(projected);
        self.call(|env, view| {
            let n = nodes.len();
            let ids = jni::objects::JLongArray::new(env, n)?;
            let roles = jni::objects::JIntArray::new(env, n)?;
            let flags = jni::objects::JIntArray::new(env, n)?;
            let actions = jni::objects::JIntArray::new(env, n)?;
            let selection = jni::objects::JIntArray::new(env, n.saturating_mul(2))?;
            let parents = jni::objects::JLongArray::new(env, n)?;
            let scroll = jni::objects::JFloatArray::new(env, n.saturating_mul(4))?;
            let bounds = jni::objects::JFloatArray::new(env, n.saturating_mul(4))?;
            let mut id_buf = vec![0i64; n];
            let mut role_buf = vec![0i32; n];
            let mut flag_buf = vec![0i32; n];
            let mut action_buf = vec![0i32; n];
            let mut selection_buf = vec![-1i32; n.saturating_mul(2)];
            let mut parent_buf = vec![0i64; n];
            let mut scroll_buf = vec![0.0f32; n.saturating_mul(4)];
            let mut bounds_buf = vec![0.0f32; n.saturating_mul(4)];
            for (index, node) in nodes.iter().enumerate() {
                id_buf[index] = node.id as i64;
                role_buf[index] = node.role;
                flag_buf[index] = node.flags as i32;
                action_buf[index] = node.actions as i32;
                if let Some((start, end)) = node.selection {
                    selection_buf[index * 2] = i32::try_from(start).unwrap_or(-1);
                    selection_buf[index * 2 + 1] = i32::try_from(end).unwrap_or(-1);
                }
                parent_buf[index] = node.parent.unwrap_or(0) as i64;
                let base = index.saturating_mul(4);
                scroll_buf[base] = node.offset_x;
                scroll_buf[base + 1] = node.offset_y;
                scroll_buf[base + 2] = node.content_w;
                scroll_buf[base + 3] = node.content_h;
                bounds_buf[base] = node.x;
                bounds_buf[base + 1] = node.y;
                bounds_buf[base + 2] = node.w;
                bounds_buf[base + 3] = node.h;
            }
            ids.set_region(env, 0, &id_buf)?;
            roles.set_region(env, 0, &role_buf)?;
            flags.set_region(env, 0, &flag_buf)?;
            actions.set_region(env, 0, &action_buf)?;
            selection.set_region(env, 0, &selection_buf)?;
            parents.set_region(env, 0, &parent_buf)?;
            scroll.set_region(env, 0, &scroll_buf)?;
            bounds.set_region(env, 0, &bounds_buf)?;
            let empty = env.new_string("")?;
            let labels = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let values = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let descriptions = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let autofill_hints = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            let autofill_values = jni::objects::JObjectArray::<JString>::new(env, n, &empty)?;
            for (index, node) in nodes.iter().enumerate() {
                if let Some(field) = &projected[index].autofill {
                    let hint = env.new_string(field.hint)?;
                    autofill_hints.set_element(env, index, &hint)?;
                    let value = env.new_string(&field.value)?;
                    autofill_values.set_element(env, index, &value)?;
                }
                let label = env.new_string(&node.label)?;
                labels.set_element(env, index, &label)?;
                let value = env.new_string(&node.value)?;
                values.set_element(env, index, &value)?;
                let description = env.new_string(&node.description)?;
                descriptions.set_element(env, index, &description)?;
            }
            env.call_method(
                view,
                jni::jni_str!("publishSemantics"),
                jni::jni_sig!(
                    "(JZ[J[I[Ljava/lang/String;[Ljava/lang/String;[I[I[I[F[Ljava/lang/String;[J[F[Ljava/lang/String;[Ljava/lang/String;)V"
                ),
                &[
                    jni::objects::JValue::Long(revision),
                    jni::objects::JValue::Bool(reset),
                    jni::objects::JValue::Object(ids.as_ref()),
                    jni::objects::JValue::Object(roles.as_ref()),
                    jni::objects::JValue::Object(labels.as_ref()),
                    jni::objects::JValue::Object(values.as_ref()),
                    jni::objects::JValue::Object(flags.as_ref()),
                    jni::objects::JValue::Object(actions.as_ref()),
                    jni::objects::JValue::Object(selection.as_ref()),
                    jni::objects::JValue::Object(scroll.as_ref()),
                    jni::objects::JValue::Object(descriptions.as_ref()),
                    jni::objects::JValue::Object(parents.as_ref()),
                    jni::objects::JValue::Object(bounds.as_ref()),
                    jni::objects::JValue::Object(autofill_hints.as_ref()),
                    jni::objects::JValue::Object(autofill_values.as_ref()),
                ],
            )?;
            Ok(())
        });
    }

    fn apply_ime(&self, call: ImeCall) {
        self.call(|env, view| {
            let text = env.new_string(&call.text)?;
            env.call_method(
                view,
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
    fn from_state(op: i32, state: &engine::session::ImeState) -> Self {
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
            input_type: input_type(engine::session::KeyboardType::Text),
            ime_action: ime_action(engine::session::TextInputAction::Done),
        }
    }
}

struct Shared(Arc<Bridge>);

impl Services for Shared {
    fn window_session(&self) -> embedder::WindowSession {
        self.0.window_session.clone()
    }

    fn restoration(&self) -> Option<embedder::Restoration> {
        Some(self.0.restoration.clone())
    }

    fn back_enabled(&self, enabled: bool) {
        let bridge = &self.0;
        bridge.back_enabled.store(enabled, Ordering::Release);
        bridge.call(|env, view| {
            env.call_method(
                view,
                jni::jni_str!("setBackEnabled"),
                jni::jni_sig!("(Z)V"),
                &[jni::objects::JValue::Bool(enabled)],
            )?;
            Ok(())
        });
    }

    fn unhandled_back(&self) {
        self.0.call(|env, view| {
            env.call_method(
                view,
                jni::jni_str!("dispatchBackFallback"),
                jni::jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        });
    }

    fn font(&self, family: &str) -> Option<&[u8]> {
        if family != FONT_FAMILY {
            return None;
        }
        self.0.font_bytes()
    }

    fn semantics(&self, update: &[embedder::ProjectedNode]) {
        let (reset, changed) = {
            let mut previous = self
                .0
                .semantics
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if previous.as_slice() == update {
                return;
            }
            let reset = previous.len() != update.len()
                || previous
                    .iter()
                    .zip(update)
                    .any(|(old, node)| old.id != node.id || old.parent != node.parent);
            let changed = if reset {
                update.to_vec()
            } else {
                previous
                    .iter()
                    .zip(update)
                    .filter(|(old, node)| old != node)
                    .map(|(_, node)| node.clone())
                    .collect()
            };
            update.clone_into(&mut previous);
            (reset, changed)
        };
        self.0.publish_semantics(&changed, reset);
    }

    fn surface_attached(&self, _id: SurfaceId) {
        let nodes = self
            .0
            .semantics
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        self.0.publish_semantics(&nodes, true);
    }

    fn source_updated(&self, revision: &str, result: Result<(), &str>) {
        self.0.update_result(revision, result);
    }

    fn platform_views(
        &self,
        surface: SurfaceId,
        placements: &[engine::tree::PlatformViewPlacement],
    ) {
        self.0.place_platform_views(surface, placements);
    }

    fn request_exit(&self) {
        self.0.finish();
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
                return vec![("SansCJK".to_string(), bytes)];
            }
        }
        Vec::new()
    }

    fn apply_ime(&self, update: &engine::session::ImeUpdate) {
        let call = match update {
            engine::session::ImeUpdate::Show(state) => ImeCall::from_state(IME_SHOW, state),
            engine::session::ImeUpdate::Update(state) => ImeCall::from_state(IME_UPDATE, state),
            engine::session::ImeUpdate::Hide => ImeCall::hide(),
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
    gestures: [i32; 4],
}

struct Window {
    native: NativeWindow,
    id: Option<SurfaceId>,
    pixels: raster::PixelSize,
}

struct Ui {
    session_ready: bool,
    window: Option<Window>,
    viewport: Option<Viewport>,
    client: Client,
    pending_user: Option<String>,
    ime_client: bool,
    ime_bottom: i32,
}

impl Ui {
    fn new() -> Self {
        Self {
            session_ready: false,
            window: None,
            viewport: None,
            client: Client::new(),
            pending_user: None,
            ime_client: false,
            ime_bottom: 0,
        }
    }
}

struct Host {
    link: embedder::Link,
    clock: Arc<FrameClock>,
    bridge: Arc<Bridge>,
    ui: Mutex<Ui>,
}

static WINDOW_SESSIONS: LazyLock<Mutex<HashMap<jlong, embedder::WindowSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static HOSTS: LazyLock<Mutex<HashMap<jlong, Arc<Host>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_HOST: AtomicI64 = AtomicI64::new(1);

fn host(handle: jlong) -> Option<Arc<Host>> {
    HOSTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&handle)
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
        EditEvent::Key(engine::session::KeyPress {
            key,
            shift,
            modifiers: engine::input::Modifiers::empty(),
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
        Edit::DeleteRange { start, end } => {
            EditEvent::Edit(engine::text::Edit::DeleteRange { start, end })
        }
        Edit::Insert(text) => EditEvent::Edit(engine::text::Edit::Insert(text)),
        Edit::Composing(text) => EditEvent::Composing(text),
    })
}

fn input_type(keyboard: engine::session::KeyboardType) -> i32 {
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
        engine::session::KeyboardType::Text => CLASS_TEXT | FLAG_AUTO_CORRECT,
        engine::session::KeyboardType::Multiline => {
            CLASS_TEXT | FLAG_MULTI_LINE | FLAG_AUTO_CORRECT
        }
        engine::session::KeyboardType::Number => CLASS_NUMBER,
        engine::session::KeyboardType::Phone => CLASS_PHONE,
        engine::session::KeyboardType::Email => CLASS_TEXT | VARIATION_EMAIL,
        engine::session::KeyboardType::Url => CLASS_TEXT | VARIATION_URI,
        engine::session::KeyboardType::Name => CLASS_TEXT | VARIATION_PERSON_NAME | FLAG_CAP_WORDS,
        engine::session::KeyboardType::Password => {
            CLASS_TEXT | VARIATION_PASSWORD | FLAG_NO_SUGGESTIONS
        }
    }
}

fn ime_action(action: engine::session::TextInputAction) -> i32 {
    match action {
        engine::session::TextInputAction::None | engine::session::TextInputAction::Newline => 1,
        engine::session::TextInputAction::Go => 2,
        engine::session::TextInputAction::Search => 3,
        engine::session::TextInputAction::Send => 4,
        engine::session::TextInputAction::Next => 5,
        engine::session::TextInputAction::Done => 6,
        engine::session::TextInputAction::Previous => 7,
    }
}

struct Attach {
    view: Global<JObject<'static>>,
    assets: Arc<AssetManager>,
    script: Option<String>,
    launch_url: Option<String>,
    saved_state: Option<String>,
    window_session: embedder::WindowSession,
    replayed_intent: bool,
    settings: engine::session::PlatformSettings,
    renderer: Renderer,
}

fn cache_log_dir(env: &mut Env, view: &JObject<'_>) -> Option<std::path::PathBuf> {
    let context = env
        .call_method(
            view,
            jni::jni_str!("getContext"),
            jni::jni_sig!("()Landroid/content/Context;"),
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    let file = env
        .call_method(
            &context,
            jni::jni_str!("getCacheDir"),
            jni::jni_sig!("()Ljava/io/File;"),
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    let path = env
        .call_method(
            &file,
            jni::jni_str!("getAbsolutePath"),
            jni::jni_sig!("()Ljava/lang/String;"),
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    let string = unsafe { JString::from_raw(env, path.as_raw()) };
    let text = string.try_to_string(env).ok()?;
    Some(std::path::PathBuf::from(text.as_str()).join("logs"))
}

fn attach(env: &mut Env, request: Attach) -> Result<jlong, String> {
    let log_dir = cache_log_dir(env, request.view.as_obj());
    let document_dir = env
        .call_method(
            request.view.as_obj(),
            jni::jni_str!("documentDirectory"),
            jni::jni_sig!("()Ljava/lang/String;"),
            &[],
        )
        .and_then(|value| value.l())
        .and_then(|value| env.cast_local::<JString>(value))
        .and_then(|value| value.try_to_string(env))
        .map_err(|error| error.to_string())?;
    let restoration = embedder::Restoration::new(request.saved_state);
    let launch_url = request
        .launch_url
        .filter(|_| !request.replayed_intent || !restoration.has_entries());
    let vm = env.get_java_vm().map_err(|error| error.to_string())?;
    let bridge = Arc::new_cyclic(|bridge: &std::sync::Weak<Bridge>| {
        let native_channel = impasto::os::NativeChannel::new({
            let bridge = std::sync::Weak::clone(bridge);
            move |id, operation, payload| {
                let Some(bridge) = bridge.upgrade() else {
                    return false;
                };
                let Ok(id) = i64::try_from(id) else {
                    return false;
                };
                let mut accepted = false;
                bridge.call(|env, view| {
                    let operation = env.new_string(operation)?;
                    let payload = env.new_string(payload)?;
                    accepted = env
                        .call_method(
                            view,
                            jni::jni_str!("dispatchNativeRequest"),
                            jni::jni_sig!("(JLjava/lang/String;Ljava/lang/String;)Z"),
                            &[
                                jni::objects::JValue::Long(id),
                                jni::objects::JValue::Object(&operation),
                                jni::objects::JValue::Object(&payload),
                            ],
                        )?
                        .z()?;
                    Ok(())
                });
                accepted
            }
        });
        let native_events = impasto::os::NativeEventChannel::new(native_channel.clone());
        Bridge {
            vm,
            view: Mutex::new(Some(Arc::new(request.view))),
            assets: Arc::clone(&request.assets),
            document_dir: (!document_dir.is_empty()).then_some(document_dir),
            font: OnceLock::new(),
            semantics: Mutex::new(Vec::new()),
            semantics_revision: AtomicI64::new(0),
            native_channel,
            native_events,
            restoration,
            window_session: request.window_session,
            back_enabled: AtomicBool::new(false),
        }
    });
    let bridge_for_clock = Arc::clone(&bridge);
    let clock = FrameClock::new(Arc::new(move |order, request| {
        bridge_for_clock.schedule_frame(order, request)
    }));
    let platform_runner = crate::main_runner::main_runner()?;
    let settings = request.settings;
    let callback = Arc::clone(&bridge);
    let boot_bridge = Arc::clone(&bridge);
    let boot_clock = Arc::clone(&clock);
    let link = embedder::start_async(
        move || {
            android_boot(
                boot_bridge,
                boot_clock,
                request.script,
                launch_url,
                log_dir,
                request.renderer,
                platform_runner,
            )
        },
        move |result| callback.session_started(result),
    )
    .map_err(|error| error.to_string())?;
    link.post(Command::Settings(settings));
    let handle = NEXT_HOST.fetch_add(1, Ordering::Relaxed);
    HOSTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            handle,
            Arc::new(Host {
                link,
                clock,
                bridge,
                ui: Mutex::new(Ui::new()),
            }),
        );
    Ok(handle)
}

fn android_boot(
    bridge: Arc<Bridge>,
    clock: Arc<FrameClock>,
    requested: Option<String>,
    initial_url: Option<String>,
    log_dir: Option<std::path::PathBuf>,
    renderer: Renderer,
    platform_runner: Arc<dyn embedder::TaskRunner>,
) -> Result<Boot, BootError> {
    let bridge_for_surface = Arc::clone(&bridge);
    let pipeline_cache = log_dir
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_owned);
    let script_assets = Arc::clone(&bridge.assets);
    let launch_url = engine::session::shared(initial_url);
    let (name, script) = load_script(&script_assets, requested.as_deref())?;
    let modules = decode::read_bytes(&script_assets, c"app.modules")
        .map(|bytes| String::from_utf8(bytes).map_err(|_| SessionError::Runtime))
        .transpose()?
        .map(|text| {
            text.split(',')
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
    let devtools = modules
        .as_ref()
        .is_none_or(|enabled| enabled.iter().any(|name| name == "devtools"));
    let trace = decode::read_bytes(&script_assets, c"app.trace").is_some();
    let ui_vm = bridge.vm.clone();
    let raster_vm = bridge.vm.clone();
    let factory: embedder::SessionFactory = Box::new(move |open, host| {
        set_display_priority(&ui_vm, jni::jni_str!("ui-boot"));
        // ActivityHost publishes the first focused resume after the view is attached.
        let host = if let Some(modules) = modules {
            host.restrict(&modules)
        } else {
            host
        };
        let mut session = if name.ends_with(".bin") {
            // SAFETY: `script` is an asset named `*.bin`. The host packages
            // that file from `impasto build`, which writes this toolchain's
            // `compile` output. It is not network input. The envelope does
            // not prove the payload is valid bytecode.
            unsafe {
                engine::js::open_trusted_bytecode(&script, &name, open.shaper, open.log, host)
            }
        } else {
            engine::js::open_program(&script, &name, open.shaper, open.log, host)
        }
        .map_err(|error| match error {
            engine::js::Error::Runtime => SessionError::Runtime,
            engine::js::Error::Script(text) => {
                tracing::error!(target: "impasto::script", "{text}");
                SessionError::Script(text)
            }
        })?;
        session.set_trace(trace);
        Ok(session)
    });
    // SAFETY: The factory keeps the shaper, VM and log callback in the JS session.
    let launch = unsafe { Launch::migrating(factory) };
    Ok(Boot {
        font_family: FONT_FAMILY.to_string(),
        launch,
        services: Box::new(Shared(Arc::clone(&bridge))),
        vsync: clock as Arc<dyn Vsync>,
        decoder: Arc::new(AssetDecoder {
            assets: Arc::clone(&bridge.assets),
        }),
        modules: Vec::new(),
        plugin_services: embedder::PluginServices {
            android: Some(embedder::AndroidContext::new(bridge.vm.clone(), {
                let bridge = Arc::downgrade(&bridge);
                move || bridge.upgrade()?.current_view()
            })),
            initial_lifecycle: impasto::Lifecycle::Inactive,
            native_channel: Some(bridge.native_channel.clone()),
            native_events: Some(bridge.native_events.clone()),
            paths: embedder::AppPaths::new(
                bridge.document_dir.as_ref().map(std::path::PathBuf::from),
                pipeline_cache.clone(),
                pipeline_cache.clone(),
            ),
        },
        platform_runner: Some(platform_runner),
        launch_url: Arc::clone(&launch_url),
        open_raster: Box::new(move |fonts| {
            set_display_priority(&raster_vm, jni::jni_str!("raster"));
            let mut raster = renderer.create_raster(fonts)?;
            if let Some(directory) = &pipeline_cache {
                // SAFETY: cache_log_dir derives this directory from Context.getCacheDir().
                unsafe { raster.set_pipeline_cache_directory(directory.clone()) };
            }
            Ok(raster)
        }),
        open_surface: Renderer::surfaces(move |id| bridge_for_surface.first_frame(id)),
        dump_list: false,
        inspect: std::env::var_os("IMPASTO_INSPECT").map(std::path::PathBuf::from),
        devtools: devtools.then(|| Arc::new(embedder::DiagnosticsHub::new())),
        devtools_source_map: None,
        inspect_dir: None,
        log_dir,
    })
}

fn set_display_priority(vm: &jni::JavaVM, name: &jni::strings::JNIStr) {
    const THREAD_PRIORITY_DISPLAY: i32 = -4;
    let result = vm.attach_current_thread_with_config(
        || jni::AttachConfig::default().thread_name(name),
        Some(8),
        |env| {
            let result = env.call_static_method(
                jni::jni_str!("android/os/Process"),
                jni::jni_str!("setThreadPriority"),
                jni::jni_sig!("(I)V"),
                &[jni::objects::JValue::Int(THREAD_PRIORITY_DISPLAY)],
            );
            if result.is_err() && env.exception_check() {
                env.exception_clear();
            }
            result.map(|_| ())
        },
    );
    if let Err(error) = result {
        tracing::debug!(target: "impasto::shell", %error, "display thread priority unavailable");
    }
}

fn detach(handle: jlong) {
    let Some(host) = HOSTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&handle)
    else {
        return;
    };
    host.bridge.native_events.close();
    host.bridge.native_channel.close();
    host.link
        .post(Command::Lifecycle(embedder::Lifecycle::Detached));
    destroy_surface(&host);
    host.link.request_shutdown();
}

fn release_window(host: &Host, keep_plugin_state: bool) {
    if !keep_plugin_state {
        host.bridge.native_events.detach_window();
    }
    let prior = host
        .bridge
        .view
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    drop(prior);
    if !keep_plugin_state {
        host.bridge.native_channel.cancel_pending();
    }
    host.link.post(Command::WindowFocus(false));
    host.link.post(Command::ImeDismissed);
    host.link
        .post(Command::Lifecycle(embedder::Lifecycle::Hidden));
    host.link.post(Command::SemanticsEnabled(false));
    host.link.post(Command::AutofillEnabled(false));
    destroy_surface(host);
    host.ui(|ui| {
        let ready = ui.session_ready;
        *ui = Ui::new();
        ui.session_ready = ready;
    });
}

fn destroy_surface(host: &Host) {
    let Some(window) = host.ui(|ui| ui.window.take()) else {
        return;
    };
    if let Some(id) = window.id {
        let _trace =
            tracing::debug_span!(target: "impasto::shell", "android_surface_destroy").entered();
        host.link.begin_destroy(id);
        // SurfaceHolder permits the producer to be torn down once its callback returns.
        host.link.drop_wait().wait(id);
    }
}

fn open_surface(host: &Host, window: NativeWindow) {
    destroy_surface(host);
    let width = window.width();
    let height = window.height();
    host.ui(|ui| {
        ui.window = Some(Window {
            native: window,
            id: None,
            pixels: pixel_size(width, height),
        });
    });
    publish_metrics(host);
}

/// Opens a drawable only once its window and logical viewport are both ready.
fn post_ready(host: &Host) -> bool {
    let ready = host.ui(|ui| {
        if !ui.session_ready {
            return None;
        }
        let window = ui.window.as_mut()?;
        if window.id.is_some() || !window.pixels.is_usable() {
            return None;
        }
        let viewport = ui.viewport.as_ref()?;
        let metrics = metrics_of(viewport)?;
        let id = SurfaceId::fresh();
        window.id = Some(id);
        Some(SurfaceTarget {
            id,
            layer: NativePtr(window.native.ptr().as_ptr().cast()),
            metrics,
            pixels: window.pixels,
        })
    });
    if let Some(target) = ready {
        host.link.post(Command::SurfaceReady(target));
        true
    } else {
        false
    }
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
    let mut metrics = Metrics::from_insets(
        width,
        height,
        viewport.density,
        engine::session::Insets {
            left: pad_left,
            top: pad_top,
            right: pad_right,
            bottom: pad_bottom,
        },
        engine::session::Insets {
            left: ime_left,
            top: ime_top,
            right: ime_right,
            bottom: ime_bottom,
        },
    );
    let [left, top, right, bottom] = viewport.gestures;
    let (left, top, right, bottom) = insets_in_points(left, top, right, bottom, viewport.density)?;
    metrics.system_gesture_insets = engine::session::Insets {
        left,
        top,
        right,
        bottom,
    };
    Some(metrics)
}

fn publish_metrics(host: &Host) {
    if post_ready(host) {
        return;
    }
    let (id, metrics, pixels) = host.ui(|ui| {
        let Some(window) = ui.window.as_ref() else {
            return (None, None, None);
        };
        let Some(viewport) = ui.viewport.as_ref() else {
            return (None, None, None);
        };
        (window.id, metrics_of(viewport), Some(window.pixels))
    });
    if let (Some(id), Some(metrics), Some(pixels)) = (id, metrics, pixels) {
        host.link.post(Command::SurfaceChanged {
            id,
            metrics,
            pixels,
        });
    }
}

fn pixel_size(width: i32, height: i32) -> raster::PixelSize {
    raster::PixelSize {
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
    }
}

fn load_script(
    assets: &AssetManager,
    requested: Option<&str>,
) -> Result<(String, Vec<u8>), SessionError> {
    let mut names = Vec::new();
    if let Some(path) = requested.and_then(motion::script_asset_path) {
        names.push(path);
    }
    names.push("app.bin".to_string());
    names.push("app.js".to_string());
    for name in names {
        let Ok(path) = CString::new(name.clone()) else {
            continue;
        };
        let Some(bytes) = decode::read_bytes(assets, &path) else {
            continue;
        };
        return Ok((name, bytes));
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

impl embedder::ImageDecoder for AssetDecoder {
    fn read_packaged_asset(&self, relative: &str, limit: usize) -> Result<Arc<[u8]>, String> {
        let path = CString::new(format!("images/{relative}")).map_err(|error| error.to_string())?;
        let asset = self
            .assets
            .open(&path)
            .ok_or_else(|| "missing packaged asset".to_owned())?;
        let length = asset.length();
        engine::session::read_asset_bytes(asset, length, limit)
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
        let paths = asset_image_path(source, scale_bucket);
        if paths.is_empty() {
            return Err(embedder::ImageError::Missing);
        }
        for path in &paths {
            let bytes = match load_asset(&self.assets, path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            return self.decode_bytes_sized(source, &bytes, target);
        }
        Err(embedder::ImageError::Missing)
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
        match decode::decode_rgba(bytes, target) {
            Ok(rgba) => Ok(image_handle(source, rgba.width, rgba.height, rgba.pixels)),
            Err(message) => Err(embedder::ImageError::Failed(message)),
        }
    }
}

fn image_handle(source: &str, width: u32, height: u32, pixels: Vec<u8>) -> embedder::ImageHandle {
    embedder::ImageHandle {
        source: Arc::from(source),
        generation: 0,
        size: [width, height],
        pixels: Arc::from(pixels),
        gpu: None,
    }
}

#[cfg(test)]
mod ime_map {
    use super::{ime_action, input_type};
    use engine::session::{KeyboardType, TextInputAction};

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
