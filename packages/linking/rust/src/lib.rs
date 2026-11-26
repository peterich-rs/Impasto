//! Inbound URL events and outbound platform links.

use std::sync::Arc;

use impasto::os::NativeChannel;
use impasto::{
    ImpastoError, ImpastoEvents, PluginContext, PluginInitError, PluginRegistrar, Runtime,
    Subscription,
};

#[derive(Debug)]
struct LinkError(String);

impl ImpastoError for LinkError {
    fn scope(&self) -> &'static str {
        "linking"
    }
    fn code(&self) -> &'static str {
        "platform"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[derive(ImpastoEvents)]
enum LinkEvent {
    Url { url: String },
}

#[impasto::module(
    name = "Linking",
    specifier = "@impasto/linking/native",
    object = true,
    init = init,
    events = LinkEvent
)]
struct LinkingPlugin {
    initial_url: Option<String>,
    channel: Option<Arc<NativeChannel>>,
    runtime: Runtime,
    _subscription: Subscription,
}

impl LinkingPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let events = ctx.emitter::<LinkEvent>();
        let subscription = ctx.on_open_url(move |url| {
            let _ = events.emit(LinkEvent::Url { url: url.into() });
        });
        Ok(Self {
            initial_url: ctx.initial_url().map(str::to_owned),
            channel: ctx.extension(),
            runtime: ctx.runtime().clone(),
            _subscription: subscription,
        })
    }
}

#[impasto::export]
impl LinkingPlugin {
    #[impasto::method(rename = "getInitialURL")]
    pub fn get_initial_url(&self) -> Option<String> {
        self.initial_url.clone()
    }

    #[impasto::method(rename = "openURL")]
    pub async fn open_url(&self, url: String) -> Result<(), LinkError> {
        if let Some(channel) = &self.channel {
            return channel
                .request("@impasto/linking/open", &url)
                .await
                .map(|_| ())
                .map_err(LinkError);
        }
        #[cfg(target_os = "macos")]
        {
            self.runtime
                .spawn_main(move || open_appkit(&url))
                .await
                .map_err(|error| LinkError(error.to_string()))?
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = &self.runtime;
            Err(LinkError(
                "opening links is unavailable on this host".into(),
            ))
        }
    }

    #[impasto::method(awaits_user)]
    pub async fn share_text(&self, text: String) -> Result<(), LinkError> {
        let channel = self
            .channel
            .as_ref()
            .ok_or_else(|| LinkError("sharing is unavailable on this host".into()))?;
        channel
            .request("@impasto/linking/share", &text)
            .await
            .map(|_| ())
            .map_err(LinkError)
    }
}

#[cfg(target_os = "macos")]
fn open_appkit(url: &str) -> Result<(), LinkError> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    let url = NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| LinkError("unsupported URL".into()))?;
    if NSWorkspace::sharedWorkspace().openURL(&url) {
        Ok(())
    } else {
        Err(LinkError("openURL failed".into()))
    }
}

/// Registers the application URL bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(LinkingPlugin::plugin());
}
