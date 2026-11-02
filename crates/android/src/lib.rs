//! Android host. The platform thread is the Java UI thread. The render thread owns the engine.

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
mod decode;
#[cfg(target_os = "android")]
mod host;
#[cfg(target_os = "android")]
mod surface;
#[cfg(target_os = "android")]
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
