//! String clipboard bindings for an Impasto window.

use std::sync::Mutex;

use impasto::{PluginContext, PluginInitError, PluginRegistrar};

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "macos")]
mod macos;

enum Clipboard {
    Memory(Mutex<String>),
    #[cfg(target_os = "android")]
    Android(impasto::os::android::AndroidContext),
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    Apple,
}

#[impasto::module(
    name = "Clipboard",
    specifier = "@impasto/clipboard/native",
    init = init,
    object = true
)]
struct ClipboardPlugin(Clipboard);

impl ClipboardPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        #[cfg(target_os = "android")]
        {
            use impasto::os::android::ContextExt;
            if let Some(platform) = ctx.android() {
                return Ok(Self(Clipboard::Android(platform.clone())));
            }
        }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if ctx.is_hosted() {
            return Ok(Self(Clipboard::Apple));
        }
        let _ = ctx;
        Ok(Self(Clipboard::Memory(Mutex::default())))
    }
}

#[cfg(target_os = "ios")]
use ios as apple;
#[cfg(target_os = "macos")]
use macos as apple;

#[impasto::export]
impl ClipboardPlugin {
    pub fn get_string(&self) -> String {
        match &self.0 {
            Clipboard::Memory(text) => text
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
            #[cfg(target_os = "android")]
            Clipboard::Android(platform) => android::get(platform),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Clipboard::Apple => apple::get(),
        }
    }

    pub fn set_string(&self, text: &str) {
        match &self.0 {
            Clipboard::Memory(value) => {
                *value.lock().unwrap_or_else(|error| error.into_inner()) = text.into()
            }
            #[cfg(target_os = "android")]
            Clipboard::Android(platform) => android::set(platform, text),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Clipboard::Apple => apple::set(text),
        }
    }
}

/// Registers the clipboard API for the application.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(ClipboardPlugin::plugin());
}
