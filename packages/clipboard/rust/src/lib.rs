//! Synchronous text and asynchronous rich media clipboard bindings.

use std::io::Cursor;
use std::sync::Mutex;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    Bytes, CancelToken, ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar,
    TypedSink,
};
use serde::{Deserialize, Serialize};

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, ImpastoType, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardFormat {
    Text,
    Html,
    Uri,
    Image,
}

#[derive(Clone, Debug, Default, ImpastoType)]
pub struct ClipboardContent {
    text: Option<String>,
    html: Option<String>,
    uri: Option<String>,
    image_png: Option<Bytes>,
}

impl ClipboardContent {
    fn formats(&self) -> Vec<ClipboardFormat> {
        [
            self.text.is_some().then_some(ClipboardFormat::Text),
            self.html.is_some().then_some(ClipboardFormat::Html),
            self.uri.is_some().then_some(ClipboardFormat::Uri),
            self.image_png.is_some().then_some(ClipboardFormat::Image),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireContent {
    text: Option<String>,
    html: Option<String>,
    uri: Option<String>,
    image_png: Option<String>,
}

#[derive(Debug)]
struct ClipboardError {
    code: &'static str,
    message: String,
}

impl ClipboardError {
    fn invalid_image() -> Self {
        Self {
            code: "invalid-image",
            message: "Clipboard images must be PNG and fit within 32 MiB".into(),
        }
    }
}

impl From<NativePluginError> for ClipboardError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "host-unavailable" => "host-unavailable",
            "empty-content" => "empty-content",
            "invalid-image" => "invalid-image",
            _ => "platform",
        };
        Self {
            code,
            message: error.message,
        }
    }
}

impl ImpastoError for ClipboardError {
    fn scope(&self) -> &'static str {
        "clipboard"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

fn validate_image(bytes: &[u8]) -> Result<(), ClipboardError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ClipboardError::invalid_image());
    }
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|_| ClipboardError::invalid_image())?;
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_IMAGE_BYTES)
        .ok_or_else(ClipboardError::invalid_image)?;
    if u64::from(reader.info().width) * u64::from(reader.info().height)
        > (MAX_IMAGE_BYTES / 4) as u64
    {
        return Err(ClipboardError::invalid_image());
    }
    reader
        .next_frame(&mut vec![0; size])
        .map_err(|_| ClipboardError::invalid_image())?;
    Ok(())
}

enum Clipboard {
    Memory(Mutex<ClipboardContent>),
    #[cfg(target_os = "android")]
    Android(impasto::os::android::AndroidContext),
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    Apple,
}

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "ios")]
use ios as apple;
#[cfg(target_os = "macos")]
use macos as apple;

#[impasto::module(name = "Clipboard", specifier = "@impasto/clipboard/native", init = init, object = true)]
struct ClipboardPlugin {
    text: Clipboard,
    native: NativePlugin,
    runtime: impasto::Runtime,
}

impl ClipboardPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            text: Self::text_driver(ctx),
            native: NativePlugin::new(ctx, "@impasto/clipboard"),
            runtime: ctx.runtime().clone(),
        })
    }

    fn text_driver(ctx: &PluginContext) -> Clipboard {
        #[cfg(target_os = "android")]
        {
            use impasto::os::android::ContextExt as _;
            if let Some(platform) = ctx.android() {
                return Clipboard::Android(platform.clone());
            }
        }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if ctx.is_hosted() {
            return Clipboard::Apple;
        }
        let _ = ctx;
        Clipboard::Memory(Mutex::default())
    }

    async fn image_io<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, ClipboardError> + Send + 'static,
    ) -> Result<T, ClipboardError> {
        if self.native.is_hosted() {
            self.runtime
                .spawn_blocking(work)
                .await
                .map_err(|error| ClipboardError::from(NativePluginError::platform(error)))?
        } else {
            work()
        }
    }
}

#[impasto::export]
impl ClipboardPlugin {
    pub fn is_clipboard_host_available(&self) -> bool {
        self.native.is_hosted()
    }

    pub fn get_string(&self) -> String {
        match &self.text {
            Clipboard::Memory(value) => value
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .text
                .clone()
                .unwrap_or_default(),
            #[cfg(target_os = "android")]
            Clipboard::Android(platform) => android::get(platform),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Clipboard::Apple => apple::get(),
        }
    }

    pub fn set_string(&self, text: &str) {
        match &self.text {
            Clipboard::Memory(value) => {
                *value.lock().unwrap_or_else(|error| error.into_inner()) = ClipboardContent {
                    text: Some(text.into()),
                    ..ClipboardContent::default()
                };
            }
            #[cfg(target_os = "android")]
            Clipboard::Android(platform) => android::set(platform, text),
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            Clipboard::Apple => apple::set(text),
        }
    }

    pub async fn available_formats(&self) -> Result<Vec<ClipboardFormat>, ClipboardError> {
        if let Clipboard::Memory(value) = &self.text {
            return Ok(value
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .formats());
        }
        Ok(self.native.request("availableFormats", &()).await?)
    }

    pub async fn has_strings(&self) -> Result<bool, ClipboardError> {
        self.has_format(ClipboardFormat::Text, "hasStrings").await
    }
    pub async fn has_images(&self) -> Result<bool, ClipboardError> {
        self.has_format(ClipboardFormat::Image, "hasImages").await
    }
    pub async fn has_html(&self) -> Result<bool, ClipboardError> {
        self.has_format(ClipboardFormat::Html, "hasHtml").await
    }

    pub async fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        if let Clipboard::Memory(value) = &self.text {
            return Ok(value
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone());
        }
        let data: WireContent = self.native.request("read", &()).await?;
        self.image_io(move || {
            let image_png = data
                .image_png
                .map(|encoded| {
                    if encoded.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                        return Err(ClipboardError::invalid_image());
                    }
                    let bytes = STANDARD
                        .decode(encoded)
                        .map_err(|_| ClipboardError::invalid_image())?;
                    validate_image(&bytes)?;
                    Ok(Bytes(bytes))
                })
                .transpose()?;
            Ok(ClipboardContent {
                text: data.text,
                html: data.html,
                uri: data.uri,
                image_png,
            })
        })
        .await
    }

    pub async fn write(&self, content: ClipboardContent) -> Result<(), ClipboardError> {
        if content.formats().is_empty() {
            return Err(ClipboardError {
                code: "empty-content",
                message: "Clipboard content needs at least one format".into(),
            });
        }
        let content = self
            .image_io(move || {
                if let Some(image) = &content.image_png {
                    validate_image(&image.0)?;
                }
                Ok(content)
            })
            .await?;
        if let Clipboard::Memory(value) = &self.text {
            *value.lock().unwrap_or_else(|error| error.into_inner()) = content;
        } else {
            let wire = self
                .image_io(move || {
                    Ok(WireContent {
                        text: content.text,
                        html: content.html,
                        uri: content.uri,
                        image_png: content.image_png.map(|image| STANDARD.encode(image.0)),
                    })
                })
                .await?;
            self.native.request::<()>("write", &wire).await?;
        }
        Ok(())
    }

    pub async fn clear(&self) -> Result<(), ClipboardError> {
        if let Clipboard::Memory(value) = &self.text {
            *value.lock().unwrap_or_else(|error| error.into_inner()) = ClipboardContent::default();
        } else {
            self.native.request::<()>("clear", &()).await?;
        }
        Ok(())
    }

    pub async fn watch_changes(
        &self,
        sink: TypedSink<Vec<ClipboardFormat>>,
        cancel: CancelToken,
    ) -> Result<(), ClipboardError> {
        if self.native.is_hosted() {
            self.native.watch_changes("change", sink, cancel).await?;
        }
        Ok(())
    }
}

impl ClipboardPlugin {
    async fn has_format(
        &self,
        format: ClipboardFormat,
        action: &str,
    ) -> Result<bool, ClipboardError> {
        if let Clipboard::Memory(value) = &self.text {
            return Ok(value
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .formats()
                .contains(&format));
        }
        Ok(self.native.request(action, &()).await?)
    }
}

/// Registers the clipboard bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(ClipboardPlugin::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_oversized_images_are_rejected_before_native_io() {
        assert_eq!(
            validate_image(b"not png").unwrap_err().code(),
            "invalid-image"
        );
        assert_eq!(
            validate_image(&vec![0; MAX_IMAGE_BYTES + 1])
                .unwrap_err()
                .code(),
            "invalid-image"
        );
    }
}
