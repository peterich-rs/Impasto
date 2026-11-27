//! Inbound deep links and validated outbound URLs.

use std::pin::pin;
use std::sync::Mutex;

use futures_channel::mpsc;
use futures_util::{StreamExt as _, future};
use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    CancelToken, ImpastoError, ImpastoEvents, ImpastoType, PluginContext, PluginInitError,
    PluginRegistrar, Subscription, TypedSink,
};
use serde::Serialize;
use url::Url;

#[derive(Debug)]
struct LinkError {
    code: &'static str,
    message: String,
}

impl LinkError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<NativePluginError> for LinkError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "host-unavailable" => "host-unavailable",
            "cannot-open" => "cannot-open",
            _ => "platform",
        };
        Self::new(code, error.message)
    }
}

impl ImpastoError for LinkError {
    fn scope(&self) -> &'static str {
        "linking"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, ImpastoType, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum OpenUrlMode {
    #[default]
    External,
    InAppBrowser,
}

#[derive(Clone, Copy, Debug, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
enum DismissButtonStyle {
    Done,
    Close,
    Cancel,
}

#[derive(Debug, Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenUrlOptions {
    mode: Option<OpenUrlMode>,
    toolbar_color: Option<String>,
    dismiss_button_style: Option<DismissButtonStyle>,
}

#[derive(Serialize)]
struct OpenUrl<'a> {
    url: &'a str,
    options: OpenUrlOptions,
}

fn validate(input: &str, mode: OpenUrlMode) -> Result<Url, LinkError> {
    if input.trim() != input || input.chars().any(char::is_control) {
        return Err(LinkError::new(
            "invalid-url",
            "URLs cannot have whitespace at the ends or control characters",
        ));
    }
    let url =
        Url::parse(input).map_err(|error| LinkError::new("invalid-url", error.to_string()))?;
    match url.scheme() {
        "file" | "javascript" | "data" | "vbscript" | "content" | "intent" | "blob" => Err(
            LinkError::new("unsupported-scheme", "this URL scheme cannot be opened"),
        ),
        "http" | "https" if !url.has_host() => {
            Err(LinkError::new("invalid-url", "a web URL requires a host"))
        }
        "http" | "https" => Ok(url),
        _ if mode == OpenUrlMode::InAppBrowser => Err(LinkError::new(
            "unsupported-scheme",
            "the in-app browser accepts HTTP or HTTPS URLs",
        )),
        _ => Ok(url),
    }
}

#[derive(ImpastoEvents)]
enum LinkEvent {
    Url { url: String },
}

#[impasto::module(name = "Linking", specifier = "@impasto/linking/native", object = true, init = init, events = LinkEvent)]
struct LinkingPlugin {
    context: PluginContext,
    native: NativePlugin,
    _subscription: Subscription,
}

impl LinkingPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let events = ctx.emitter::<LinkEvent>();
        let subscription = ctx.on_open_url(move |url| {
            let _ = events.emit(LinkEvent::Url { url: url.into() });
        });
        Ok(Self {
            context: ctx.clone(),
            native: NativePlugin::new(ctx, "@impasto/linking"),
            _subscription: subscription,
        })
    }
}

#[impasto::export]
impl LinkingPlugin {
    pub fn is_linking_host_available(&self) -> bool {
        self.native.is_hosted()
    }

    #[impasto::method(rename = "getInitialURL")]
    pub fn get_initial_url(&self) -> Option<String> {
        self.context.initial_url().map(str::to_owned)
    }

    #[impasto::method(rename = "canOpenURL")]
    pub async fn can_open_url(&self, url: String) -> Result<bool, LinkError> {
        let url = validate(&url, OpenUrlMode::External)?;
        if !self.native.is_hosted() {
            return Ok(false);
        }
        #[cfg(target_os = "macos")]
        if !self.native.has_channel() {
            return self
                .context
                .runtime()
                .spawn_main(move || open_appkit(url.as_str(), false))
                .await
                .map_err(|error| LinkError::new("host-unavailable", error.to_string()));
        }
        Ok(self.native.request("canOpen", &url.as_str()).await?)
    }

    #[impasto::method(rename = "openURL", awaits_user)]
    pub async fn open_url(
        &self,
        url: String,
        options: Option<OpenUrlOptions>,
    ) -> Result<bool, LinkError> {
        let options = options.unwrap_or_default();
        let url = validate(&url, options.mode.unwrap_or_default())?;
        if options.toolbar_color.as_ref().is_some_and(|value| {
            value.len() != 7
                || !value.starts_with('#')
                || !value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        }) {
            return Err(LinkError::new(
                "invalid-url",
                "toolbarColor must use #RRGGBB",
            ));
        }
        #[cfg(target_os = "macos")]
        if self.native.is_hosted() && !self.native.has_channel() {
            return self
                .context
                .runtime()
                .spawn_main(move || open_appkit(url.as_str(), true))
                .await
                .map_err(|error| LinkError::new("host-unavailable", error.to_string()));
        }
        Ok(self
            .native
            .request(
                "open",
                &OpenUrl {
                    url: url.as_str(),
                    options,
                },
            )
            .await?)
    }

    pub async fn watch_urls(
        &self,
        mut sink: TypedSink<String>,
        cancel: CancelToken,
    ) -> Result<(), LinkError> {
        let (sender, mut receiver) = mpsc::channel(16);
        let sender = Mutex::new(sender);
        let _subscription = self.context.on_open_url(move |url| {
            if let Ok(mut sender) = sender.lock()
                && let Err(error) = sender.try_send(url.to_owned())
                && error.is_full()
            {
                impasto::tracing::warn!("deep-link stream is full");
            }
        });
        let forward = async {
            while let Some(url) = receiver.next().await {
                if sink.send(url).await.is_err() {
                    break;
                }
            }
        };
        future::select(pin!(forward), pin!(cancel.cancelled())).await;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn open_appkit(url: &str, open: bool) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    NSURL::URLWithString(&NSString::from_str(url)).is_some_and(|url| {
        let workspace = NSWorkspace::sharedWorkspace();
        if open {
            workspace.openURL(&url)
        } else {
            workspace.URLForApplicationToOpenURL(&url).is_some()
        }
    })
}

/// Registers the application URL bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(LinkingPlugin::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_validation_rejects_local_and_script_url_schemes() {
        for input in [
            "file:///tmp/test",
            "javaScript:alert(1)",
            "content://foo",
            "data:text/plain,x",
        ] {
            assert_eq!(
                validate(input, OpenUrlMode::External).unwrap_err().code(),
                "unsupported-scheme"
            );
        }
        assert_eq!(
            validate("https://", OpenUrlMode::External)
                .unwrap_err()
                .code(),
            "invalid-url"
        );
        assert!(validate("demo://callback", OpenUrlMode::External).is_ok());
        assert!(validate("demo://callback", OpenUrlMode::InAppBrowser).is_err());
    }
}
