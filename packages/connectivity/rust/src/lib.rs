//! Network transport and system data-use state for the current window.

use futures_util::{Stream, StreamExt as _};
use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    CancelToken, ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar,
    TypedSink,
};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, ImpastoType, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectivityType {
    Wifi,
    Cellular,
    Ethernet,
    Vpn,
    Bluetooth,
    Other,
    None,
}

#[derive(Clone, Debug, Deserialize, Eq, ImpastoType, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityState {
    pub types: Vec<ConnectivityType>,
    pub is_connected: bool,
    pub is_expensive: bool,
    pub is_constrained: bool,
}

#[derive(Debug)]
pub struct ConnectivityError(NativePluginError);

impl ImpastoError for ConnectivityError {
    fn scope(&self) -> &'static str {
        "connectivity"
    }
    fn code(&self) -> &'static str {
        if self.0.code == "host-unavailable" {
            "host-unavailable"
        } else {
            "platform"
        }
    }
    fn message(&self) -> String {
        self.0.message.clone()
    }
}

fn platform(ctx: &PluginContext) -> NativePlugin {
    NativePlugin::new(ctx, "@impasto/connectivity")
}

async fn status(platform: &NativePlugin) -> Result<ConnectivityState, ConnectivityError> {
    if !platform.is_hosted() {
        return Ok(ConnectivityState {
            types: vec![ConnectivityType::None],
            is_connected: false,
            is_expensive: false,
            is_constrained: false,
        });
    }
    platform
        .request("status", &())
        .await
        .map_err(ConnectivityError)
}

/// Reads the current connection state.
pub async fn current_status(ctx: &PluginContext) -> Result<ConnectivityState, ConnectivityError> {
    status(&platform(ctx)).await
}

/// Observes network changes. Drop the stream to release the native subscription.
pub async fn watch_status(
    ctx: &PluginContext,
) -> Result<
    impl Stream<Item = Result<ConnectivityState, ConnectivityError>> + use<>,
    ConnectivityError,
> {
    let stream = platform(ctx)
        .subscribe_latest("status")
        .await
        .map_err(ConnectivityError)?;
    let mut previous = None;
    Ok(stream.filter_map(move |payload| {
        let state: Result<ConnectivityState, _> = serde_json::from_str(&payload)
            .map_err(|error| ConnectivityError(NativePluginError::platform(error)));
        let duplicate = state
            .as_ref()
            .is_ok_and(|state| Some(state) == previous.as_ref());
        if let Ok(state) = &state {
            previous = Some(state.clone());
        }
        futures_util::future::ready((!duplicate).then_some(state))
    }))
}

#[impasto::module(name = "Connectivity", specifier = "@impasto/connectivity/native", init = init)]
struct ConnectivityPlugin(NativePlugin);

impl ConnectivityPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self(platform(ctx)))
    }
}

#[impasto::export]
impl ConnectivityPlugin {
    pub fn is_connectivity_host_available(&self) -> bool {
        self.0.is_hosted()
    }

    pub async fn get_status(&self) -> Result<ConnectivityState, ConnectivityError> {
        status(&self.0).await
    }

    pub async fn watch_status(
        &self,
        mut sink: TypedSink<ConnectivityState>,
        cancel: CancelToken,
    ) -> Result<(), ConnectivityError> {
        if self.0.is_hosted() {
            return self
                .0
                .watch("status", sink, cancel)
                .await
                .map_err(ConnectivityError);
        }
        let _ = sink.send(status(&self.0).await?).await;
        Ok(())
    }
}

/// Registers the connectivity capability.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(ConnectivityPlugin::plugin());
}
