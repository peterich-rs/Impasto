//! JNI methods declared by Android's Kotlin NativeBridge.

#![cfg_attr(
    feature = "app-entry",
    allow(
        non_snake_case,
        reason = "the generated application entry forwards the same JNI method names"
    )
)]

use std::ffi::c_char;
use std::ptr::NonNull;
use std::sync::Arc;

use embedder::{Command, FrameToken, Lifecycle};
use engine::session::{InputEvent as EditEvent, Key};
use jni::objects::{JDoubleArray, JIntArray, JObject, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};
use ndk::asset::AssetManager;
use ndk::native_window::NativeWindow;

use super::{
    Attach, IME_HIDE, IME_UPDATE, Viewport, attach, destroy_surface, detach, edit_event, host,
    open_surface, pixel_size, publish_metrics, release_window,
};
use crate::insets::scene_point;
use crate::motion::android_wheel;
use crate::surface::Renderer;
use crate::textinput::{self, KeyOutcome};

unsafe extern "C" {
    fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
}

fn settings_from(
    locale: String,
    text_scale: f32,
    dark: bool,
    reduce_motion: bool,
    bold_text: bool,
    high_contrast: bool,
) -> engine::session::PlatformSettings {
    engine::session::PlatformSettings {
        locale,
        text_scale: if text_scale.is_finite() && text_scale > 0.0 {
            text_scale
        } else {
            1.0
        },
        brightness: if dark {
            engine::session::Brightness::Dark
        } else {
            engine::session::Brightness::Light
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
    // SAFETY: NativeBridge supplies the Activity AssetManager. Its application owns the manager.
    let raw = unsafe { ndk_sys::AAssetManager_fromJava(env.get_raw().cast(), assets.as_raw()) };
    let Some(ptr) = NonNull::new(raw) else {
        return Err("asset manager was null".to_string());
    };
    Ok(Arc::new(unsafe { AssetManager::from_ptr(ptr) }))
}

/// Logcat before the process subscriber is installed. Later logs go through `tracing`.
pub(super) fn early_log(message: &str) {
    let mut bytes = message.replace('\0', " ").into_bytes();
    bytes.push(0);
    unsafe {
        __android_log_write(6, c"impasto".as_ptr(), bytes.as_ptr().cast());
    }
}

fn with_env<'local, T: Default>(
    env: &mut EnvUnowned<'local>,
    body: impl FnOnce(&mut Env<'local>) -> jni::errors::Result<T>,
) -> T {
    env.with_env(body)
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeCreateWindowSession(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
) -> jlong {
    let id = super::NEXT_HOST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    super::WINDOW_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(id, embedder::WindowSession::default());
    id
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeCloseWindowSession(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    id: jlong,
) {
    super::WINDOW_SESSIONS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&id);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeAttach<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    view: JObject<'local>,
    assets: JObject<'local>,
    script: JString<'local>,
    launch_url: JString<'local>,
    saved_state: JString<'local>,
    replayed_intent: jboolean,
    renderer: JString<'local>,
    locale: JString<'local>,
    text_scale: f32,
    dark: jboolean,
    reduce_motion: jboolean,
    bold_text: jboolean,
    high_contrast: jboolean,
    window_session_id: jlong,
) -> jlong {
    with_env(&mut env, |env| {
        let script = java_string(env, &script)?;
        let launch_url = crate::nonempty_url(java_string(env, &launch_url)?.as_deref());
        let saved_state = java_string(env, &saved_state)?;
        let renderer = match java_string(env, &renderer)?.as_deref() {
            None | Some("auto") => Renderer::Auto,
            Some("vulkan") => Renderer::Vulkan,
            Some("gles") => Renderer::Gles,
            Some(other) => {
                early_log(&format!("unknown Android renderer {other}"));
                return Ok(0);
            }
        };
        let locale = java_string(env, &locale)?.unwrap_or_default();
        let assets = match asset_manager(env, &assets) {
            Ok(assets) => assets,
            Err(message) => {
                early_log(&message);
                return Ok(0);
            }
        };
        let window_session = if window_session_id == 0 {
            embedder::WindowSession::default()
        } else {
            let sessions = super::WINDOW_SESSIONS
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(session) = sessions.get(&window_session_id) else {
                return Ok(0);
            };
            session.clone()
        };
        let view = env.new_global_ref(&view)?;
        let request = Attach {
            view,
            assets,
            script,
            launch_url,
            saved_state,
            window_session,
            replayed_intent,
            renderer,
            settings: settings_from(
                locale,
                text_scale,
                dark,
                reduce_motion,
                bold_text,
                high_contrast,
            ),
        };
        match attach(env, request) {
            Ok(handle) => Ok(handle),
            Err(message) => {
                early_log(&message);
                Ok(0)
            }
        }
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSavedState<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
) -> JString<'local> {
    with_env(&mut env, |env| {
        host(handle)
            .and_then(|host| host.bridge.restoration.saved_state())
            .map_or_else(|| Ok(JString::null()), |state| env.new_string(state))
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativePlatformPending(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    request: jlong,
) -> jboolean {
    host(handle).is_some_and(|host| {
        u64::try_from(request).is_ok_and(|id| host.bridge.native_channel.contains(id))
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeDetach(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    detach(handle);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeReleaseWindow(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    keep_plugin_state: jboolean,
) {
    if let Some(host) = host(handle) {
        release_window(&host, keep_plugin_state);
    }
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeAttachWindow<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    view: JObject<'local>,
) -> jboolean {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(false);
        };
        let view = Arc::new(env.new_global_ref(&view)?);
        {
            let mut slot = host
                .bridge
                .view
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if slot.is_some() {
                return Ok(false);
            }
            *slot = Some(Arc::clone(&view));
        }
        let enabled = host
            .bridge
            .back_enabled
            .load(std::sync::atomic::Ordering::Acquire);
        env.call_method(
            view.as_obj(),
            jni::jni_str!("setBackEnabled"),
            jni::jni_sig!("(Z)V"),
            &[jni::objects::JValue::Bool(enabled)],
        )?;
        Ok(true)
    })
}

fn valid_texture_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeHardwareTexture<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    name: JString<'local>,
    width: jint,
    height: jint,
    image: JObject<'local>,
) -> jboolean {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(false);
        };
        let Some(name) = java_string(env, &name)?.filter(|name| valid_texture_name(name)) else {
            return Ok(false);
        };
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return Ok(false);
        };
        if image.is_null() || !(1..=2_073_600).contains(&(u64::from(width) * u64::from(height))) {
            return Ok(false);
        }
        let frame = Arc::new(super::hardware_frame::HardwareFrame::new(
            env.get_java_vm()?,
            env.new_global_ref(&image)?,
            raster::PixelSize::new(width, height),
        ));
        host.link.post(Command::ExternalGpuTexture {
            source: format!("external:{name}"),
            frame: Some(Box::new(move |raster| frame.image(raster))),
        });
        Ok(true)
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeExternalTexture<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    name: JString<'local>,
    width: jint,
    height: jint,
    bytes: jni::objects::JByteArray<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        let Some(name) = java_string(env, &name)? else {
            return Ok(());
        };
        if !valid_texture_name(&name) {
            return Ok(());
        }
        let frame = if bytes.is_null() {
            None
        } else {
            let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
                return Ok(());
            };
            let count = u64::from(width) * u64::from(height);
            if !(1..=2_073_600).contains(&count) || bytes.len(env)? as u64 != count * 4 {
                return Ok(());
            }
            Some(embedder::ExternalFrame {
                size: [width, height],
                pixels: env.convert_byte_array(&bytes)?,
            })
        };
        host.link.post(Command::ExternalTexture {
            source: format!("external:{name}"),
            frame,
        });
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeUpdateSource<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    revision: JString<'local>,
    source: jni::objects::JByteArray<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        let Some(revision) = java_string(env, &revision)? else {
            return Ok(());
        };
        if !(1..=20).contains(&revision.len())
            || !revision.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Ok(());
        }
        let length = source.len(env)?;
        if !(1..=16 * 1024 * 1024).contains(&length) {
            host.bridge
                .update_result(&revision, Err("invalid source size"));
            return Ok(());
        }
        match String::from_utf8(env.convert_byte_array(&source)?) {
            Ok(source) => host.link.post(Command::UpdateSource { revision, source }),
            Err(_) => host
                .bridge
                .update_result(&revision, Err("source is not UTF-8")),
        }
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSessionReady(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    let first = host.ui(|ui| !std::mem::replace(&mut ui.session_ready, true));
    if first {
        publish_metrics(&host);
    }
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativePlatformResult<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    request: jlong,
    payload: JString<'local>,
    error: JString<'local>,
) -> jboolean {
    with_env(&mut env, |env| {
        let (Some(host), Ok(id)) = (host(handle), u64::try_from(request)) else {
            return Ok(false);
        };
        let result = if let Some(error) = java_string(env, &error)? {
            Err(error)
        } else {
            Ok(java_string(env, &payload)?)
        };
        Ok(host.bridge.native_channel.complete(id, result))
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeIsCurrentSurface(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    surface: jlong,
) -> jboolean {
    host(handle).is_some_and(|host| {
        host.ui(|ui| {
            ui.window.as_ref().and_then(|window| window.id)
                == Some(embedder::SurfaceId(surface as u64))
        })
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeResume(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::Lifecycle(Lifecycle::Resumed));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeInactive(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::Lifecycle(Lifecycle::Inactive));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeHidden(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::Lifecycle(Lifecycle::Hidden));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeTrimMemory(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::MemoryWarning);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeDismissIme(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    if let Some(host) = host(handle) {
        host.link.post(Command::ImeDismissed);
    }
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeFocus(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    focused: jboolean,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::WindowFocus(focused));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeBack(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
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

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeBackGesture(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    phase: jint,
    value: f32,
) {
    use engine::session::BackGesture;
    let Some(host) = host(handle) else {
        return;
    };
    let event = match phase {
        0 => BackGesture::Started {
            from_right: value > 0.0,
        },
        1 if value.is_finite() => BackGesture::Progress(value.clamp(0.0, 1.0)),
        2 => BackGesture::Cancelled,
        3 => BackGesture::Committed,
        _ => return,
    };
    host.link.post(Command::BackGesture(event));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeWindowDisplay<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    display_id: jint,
    refresh_rate: f32,
    density: f32,
    features: JIntArray<'local>,
) {
    use engine::session::{DisplayFeature, DisplayFeatureKind, FoldPosture, WindowDisplay};
    let Some(host) = host(handle) else { return };
    let features = with_env(&mut env, |env| {
        let length = features.len(env)?;
        if length > 224 || length % 7 != 0 || !density.is_finite() || density <= 0.0 {
            return Ok(Vec::new());
        }
        let mut values = vec![0; length];
        features.get_region(env, 0, &mut values)?;
        Ok(values
            .chunks_exact(7)
            .filter_map(|feature| {
                let [x, y, w, h, kind, posture, separating] = *feature else {
                    return None;
                };
                if x < 0 || y < 0 || w < 0 || h < 0 || (w == 0 && h == 0) {
                    return None;
                }
                Some(DisplayFeature {
                    bounds: engine::Rect {
                        x: x as f32 / density,
                        y: y as f32 / density,
                        w: w as f32 / density,
                        h: h as f32 / density,
                    },
                    kind: match kind {
                        1 => DisplayFeatureKind::Fold,
                        2 => DisplayFeatureKind::Hinge,
                        3 => DisplayFeatureKind::Cutout,
                        _ => return None,
                    },
                    posture: match posture {
                        1 => Some(FoldPosture::Flat),
                        2 => Some(FoldPosture::HalfOpened),
                        _ => None,
                    },
                    separating: separating != 0,
                })
            })
            .collect())
    });
    host.link.post(Command::WindowDisplay(WindowDisplay {
        display_id: (display_id >= 0).then_some(display_id),
        refresh_rate: (refresh_rate.is_finite() && refresh_rate > 0.0).then_some(refresh_rate),
        features,
    }));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSettings<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    locale: JString<'local>,
    text_scale: f32,
    dark: jboolean,
    reduce_motion: jboolean,
    bold_text: jboolean,
    high_contrast: jboolean,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        let locale = java_string(env, &locale)?.unwrap_or_default();
        host.link.post(Command::Settings(settings_from(
            locale,
            text_scale,
            dark,
            reduce_motion,
            bold_text,
            high_contrast,
        )));
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeLaunchUrl<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    url: JString<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        if let Some(url) = crate::nonempty_url(java_string(env, &url)?.as_deref()) {
            host.link.post(Command::LaunchUrl(url));
        }
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeInvalidate(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    if let Some(host) = host(handle) {
        host.link.post(Command::Invalidate);
    }
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSurfaceCreated<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    surface: JObject<'local>,
) -> jboolean {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(false);
        };
        // SAFETY: The Kotlin holder supplies a live Surface. Android returns an acquired reference.
        let raw =
            unsafe { ndk_sys::ANativeWindow_fromSurface(env.get_raw().cast(), surface.as_raw()) };
        let Some(ptr) = NonNull::new(raw) else {
            tracing::error!(target: "impasto::shell", "surface produced no native window");
            return Ok(false);
        };
        // SAFETY: The acquired reference is owned by this wrapper until the renderer releases it.
        let window = unsafe { NativeWindow::from_ptr(ptr) };
        open_surface(&host, window);
        Ok(true)
    })
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSurfaceChanged(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    width: jint,
    height: jint,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.ui(|ui| {
        let pixels = pixel_size(width, height);
        if let Some(window) = ui.window.as_mut()
            && pixels != window.pixels
        {
            window.pixels = pixels;
        }
    });
    publish_metrics(&host);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSurfaceDestroyed(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    destroy_surface(&host);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeMetrics(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
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
    gesture_left: jint,
    gesture_top: jint,
    gesture_right: jint,
    gesture_bottom: jint,
) {
    let Some(host) = host(handle) else {
        return;
    };
    let dismissed = host.ui(|ui| {
        let dismissed = ui.ime_bottom > 0 && ime_bottom == 0;
        ui.ime_bottom = ime_bottom;
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
            gestures: [gesture_left, gesture_top, gesture_right, gesture_bottom],
        });
        dismissed
    });
    if dismissed {
        host.link.post(Command::ImeDismissed);
    }
    publish_metrics(&host);
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativePointer<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    phase: jint,
    pointer_id: jlong,
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
    positions: JDoubleArray<'local>,
    history_count: jint,
) {
    use engine::input::{PointerHistory, PointerSample};

    let Some(host) = host(handle) else {
        return;
    };
    let (x, y) = match scene_point(x, y, density) {
        Some(point) => point,
        None if phase == 3 => (f32::NAN, f32::NAN),
        None => return,
    };
    let kind = match kind {
        1 => engine::input::PointerKind::Mouse,
        2 => engine::input::PointerKind::Pen,
        _ => engine::input::PointerKind::Touch,
    };
    let button = match button {
        1 => engine::input::Button::Primary,
        2 => engine::input::Button::Secondary,
        3 => engine::input::Button::Auxiliary,
        _ => engine::input::Button::None,
    };
    if phase == 5 {
        let Some((dx, dy)) = android_wheel(wheel_x, wheel_y) else {
            return;
        };
        let mut event = engine::input::PointerEvent::wheel(x, y, time_ms as f64, dx, dy);
        event.pointer = engine::input::PointerId(pointer_id as u64);
        event.kind = kind;
        event.modifiers = engine::input::Modifiers::from_bits(modifiers as u8);
        host.link.post(Command::Pointer(event));
        return;
    }
    let (x, y) = if phase == 6 { (-1.0, -1.0) } else { (x, y) };
    let phase = match phase {
        0 => engine::input::Phase::Down,
        1 | 4 | 6 => engine::input::Phase::Move,
        2 => engine::input::Phase::Up,
        3 => engine::input::Phase::Cancel,
        _ => return,
    };
    let button = if phase == engine::input::Phase::Cancel {
        engine::input::Button::None
    } else if kind == engine::input::PointerKind::Touch && button == engine::input::Button::None {
        engine::input::Button::Primary
    } else {
        button
    };
    let event = engine::input::PointerEvent {
        pointer: engine::input::PointerId(pointer_id as u64),
        kind,
        phase,
        x,
        y,
        time_ms: time_ms as f64,
        button,
        buttons: engine::input::Buttons::from_bits(buttons as u8),
        modifiers: engine::input::Modifiers::from_bits(modifiers as u8),
        dx: 0.0,
        dy: 0.0,
        zoom: 0.0,
        pen: None,
    };
    if phase == engine::input::Phase::Move
        && let Ok(count) = usize::try_from(history_count)
        && (1..=PointerHistory::CAPACITY).contains(&count)
    {
        let mut packed = [0.0; PointerHistory::CAPACITY * 3];
        let values = &mut packed[..count * 3];
        let read = with_env(&mut env, |env| {
            if positions.len(env)? < values.len() {
                return Ok(false);
            }
            positions.get_region(env, 0, values)?;
            Ok(true)
        });
        if read {
            let mut history = Box::new(PointerHistory::default());
            for &[x, y, time_ms] in values.as_chunks::<3>().0 {
                if let Some((x, y)) = scene_point(x as f32, y as f32, density)
                    && time_ms.is_finite()
                    && time_ms < event.time_ms
                {
                    history.push(PointerSample { x, y, time_ms });
                }
            }
            host.link.post(Command::PointerMoves { event, history });
            return;
        }
    }
    host.link.post(Command::Pointer(event));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeZoom(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    scale: f32,
    x: f32,
    y: f32,
    time_ms: jlong,
    density: f32,
) {
    let Some(host) = host(handle) else {
        return;
    };
    let Some((x, y)) = scene_point(x, y, density) else {
        return;
    };
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let mut event = engine::input::PointerEvent::zoom(x, y, time_ms as f64, scale);
    event.kind = engine::input::PointerKind::Touch;
    host.link.post(Command::Pointer(event));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeKey(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    action: jint,
    key_code: jint,
    meta: jint,
    unicode: jint,
) {
    let Some(host) = host(handle) else {
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

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSync<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    text: JString<'local>,
    sel_start: jint,
    sel_end: jint,
    comp_start: jint,
    comp_end: jint,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
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

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeAction(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.clear_pending();
    host.link.post(Command::Input(EditEvent::Key(
        engine::session::KeyPress::new(Key::Enter),
    )));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeContext(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    kind: jint,
) {
    let Some(host) = host(handle) else {
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

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeAccept<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    op: jint,
    text: JString<'local>,
    sel_start: jint,
    sel_end: jint,
    comp_start: jint,
    comp_end: jint,
) -> jboolean {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
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
            if op == IME_UPDATE
                && ui
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

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeOnVsync(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    token: jlong,
    frame_time_nanos: jlong,
    target_nanos: jlong,
    interval_nanos: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.clock.on_vsync(
        FrameToken(token as u64),
        frame_time_nanos,
        target_nanos,
        interval_nanos,
    );
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSemanticsEnabled(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    enabled: jboolean,
    autofill: jboolean,
) {
    if let Some(host) = host(handle) {
        host.link.post(Command::SemanticsEnabled(enabled));
        host.link.post(Command::AutofillEnabled(autofill));
    }
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeActivate(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    id: jlong,
) {
    let Some(host) = host(handle) else {
        return;
    };
    host.link.post(Command::SemanticsAction(
        engine::semantics::SemanticsAction::Activate(engine::semantics::SemanticsId(id as u64)),
    ));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeScroll(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    id: jlong,
    forward: jboolean,
) {
    let Some(host) = host(handle) else {
        return;
    };
    let id = engine::semantics::SemanticsId(id as u64);
    let (dx, dy) = {
        let nodes = host
            .bridge
            .semantics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(node) = nodes.iter().find(|node| node.id == id) else {
            return;
        };
        if forward {
            node.scroll_forward
        } else {
            node.scroll_backward
        }
    };
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    host.link.post(Command::SemanticsAction(
        engine::semantics::SemanticsAction::Scroll { id, dx, dy },
    ));
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSetText<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    id: jlong,
    text: JString<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        let text = java_string(env, &text)?.unwrap_or_default();
        host.link.post(Command::SemanticsAction(
            engine::semantics::SemanticsAction::SetText {
                id: engine::semantics::SemanticsId(id as u64),
                text,
            },
        ));
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeAutofill<'local>(
    mut env: EnvUnowned<'local>,
    _class: JObject<'local>,
    handle: jlong,
    id: jlong,
    text: JString<'local>,
) {
    with_env(&mut env, |env| {
        let Some(host) = host(handle) else {
            return Ok(());
        };
        let text = java_string(env, &text)?.unwrap_or_default();
        host.link.post(Command::SemanticsAction(
            engine::semantics::SemanticsAction::Autofill {
                id: engine::semantics::SemanticsId(id as u64),
                text,
            },
        ));
        Ok(())
    });
}

#[cfg_attr(not(feature = "app-entry"), unsafe(no_mangle))]
pub extern "system" fn Java_dev_impasto_host_NativeBridge_nativeSetSelection(
    _env: EnvUnowned<'_>,
    _class: JObject<'_>,
    handle: jlong,
    id: jlong,
    start: jint,
    end: jint,
) {
    if let (Some(host), Ok(start), Ok(end)) =
        (host(handle), u32::try_from(start), u32::try_from(end))
    {
        host.link.post(Command::SemanticsAction(
            engine::semantics::SemanticsAction::SetSelection {
                id: engine::semantics::SemanticsId(id as u64),
                start,
                end,
            },
        ));
    }
}
