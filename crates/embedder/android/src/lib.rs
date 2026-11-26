//! Android host. The platform thread is Android's main thread. The render thread owns the engine.

#![allow(
    unsafe_code,
    reason = "JNI, ANativeWindow, and the NDK are called from this crate"
)]

#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "the JNI host is the only caller")
)]
mod a11y;
#[cfg_attr(
    not(target_os = "android"),
    allow(
        dead_code,
        reason = "host.rs is the caller and is compiled only for Android"
    )
)]
mod assets;

#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "the JNI host is the only caller")
)]
mod insets;
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "the JNI host is the only caller")
)]
mod motion;
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "the JNI host is the only caller")
)]
mod textinput;

#[cfg(target_os = "android")]
mod app;
#[cfg(target_os = "android")]
mod decode;
#[cfg(target_os = "android")]
mod main_runner;
#[cfg(target_os = "android")]
mod surface;
#[cfg(any(target_os = "android", test))]
mod vsync;

/// Trimmed Intent data. Missing or empty is not a launch URL.
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "the JNI host is the only caller")
)]
pub(crate) fn nonempty_url(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::nonempty_url;

    #[test]
    fn skips_missing_and_empty() {
        assert_eq!(nonempty_url(None), None);
        assert_eq!(nonempty_url(Some("")), None);
        assert_eq!(nonempty_url(Some("  ")), None);
        assert_eq!(
            nonempty_url(Some("https://app.example/item")),
            Some("https://app.example/item".into())
        );
        assert_eq!(
            nonempty_url(Some(" file:///tmp/a ")),
            Some("file:///tmp/a".into())
        );
    }
}

#[cfg(all(target_os = "android", feature = "app-entry"))]
pub use embedder::install_plugins;

#[cfg(all(target_os = "android", feature = "app-entry"))]
#[doc(hidden)]
pub mod app_jni {
    pub use crate::app::entrypoints::*;
    pub use jni;
    pub use jni::{
        EnvUnowned,
        objects::{JIntArray, JObject, JObjectArray, JString},
        sys::{jboolean, jint, jlong},
    };
}

#[cfg(all(
    target_os = "android",
    feature = "bundled-plugins",
    not(feature = "app-entry")
))]
#[expect(unsafe_code, reason = "JNI loads this process entry point by symbol")]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> jni::sys::jint {
    if embedder::install_bundled_plugins(impasto_bundled_plugins::register).is_ok() {
        0x0001_0006
    } else {
        -1
    }
}
