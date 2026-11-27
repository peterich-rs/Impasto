//! JSON messages shared by Swift and Kotlin capability plugins.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use futures_core::Stream;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::os::{NativeChannel, NativeEventChannel, NativeEventStream};
use crate::{CancelToken, IntoPluginValue, PluginContext, TypedSink};

/// The request and event channels of one capability in a window session.
#[derive(Clone)]
pub struct NativePlugin {
    namespace: &'static str,
    requests: Option<Arc<NativeChannel>>,
    events: Option<Arc<NativeEventChannel>>,
    hosted: bool,
}

/// A failed platform operation and its wire error code.
#[derive(Clone, Debug, Deserialize)]
pub struct NativePluginError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for NativePluginError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.message)
    }
}

impl std::error::Error for NativePluginError {}

impl NativePluginError {
    pub fn platform(error: impl std::fmt::Display) -> Self {
        Self {
            code: "platform".into(),
            message: error.to_string(),
        }
    }

    fn host_unavailable(message: &str) -> Self {
        Self {
            code: "host-unavailable".into(),
            message: message.into(),
        }
    }

    fn decode(message: String) -> Self {
        match message.as_str() {
            "the platform session is closed"
            | "the platform window detached"
            | "Platform window is detached" => Self {
                code: "cancelled".into(),
                message,
            },
            "the platform window is unavailable" => Self::host_unavailable(&message),
            _ => serde_json::from_str(&message).unwrap_or_else(|_| Self::platform(message)),
        }
    }
}

impl NativePlugin {
    /// Binds a plugin namespace to the channels available in this context.
    pub fn new(ctx: &PluginContext, namespace: &'static str) -> Self {
        Self {
            namespace,
            requests: ctx.extension(),
            events: ctx.extension(),
            hosted: ctx.is_hosted(),
        }
    }

    /// Reports whether mocks must be rejected for this context.
    pub fn is_hosted(&self) -> bool {
        self.hosted
    }

    /// Reports whether the platform provides a native request channel.
    pub fn has_channel(&self) -> bool {
        self.requests.is_some()
    }

    /// Exchanges a JSON value with a platform plugin. A nullable C result decodes as JSON null.
    pub async fn request<T: DeserializeOwned>(
        &self,
        action: &str,
        payload: &impl Serialize,
    ) -> Result<T, NativePluginError> {
        let channel = self.requests.as_ref().ok_or_else(|| {
            NativePluginError::host_unavailable(
                "the platform plugin requires a native application host",
            )
        })?;
        let operation = format!("{}/{action}", self.namespace);
        let payload = serde_json::to_string(payload).map_err(NativePluginError::platform)?;
        let reply = channel
            .request(&operation, &payload)
            .await
            .map_err(NativePluginError::decode)?;
        serde_json::from_str(reply.as_deref().unwrap_or("null"))
            .map_err(NativePluginError::platform)
    }

    /// Subscribes to a bounded topic that carries the latest available state.
    pub async fn subscribe_latest(
        &self,
        event: &str,
    ) -> Result<NativeEventStream, NativePluginError> {
        self.event_channel()?
            .subscribe_latest(format!("{}/{event}", self.namespace), "", 4)
            .await
            .map_err(NativePluginError::decode)
    }

    fn event_channel(&self) -> Result<&NativeEventChannel, NativePluginError> {
        self.events.as_deref().ok_or_else(|| {
            NativePluginError::host_unavailable(
                "the platform plugin requires a native event channel",
            )
        })
    }

    /// Forwards native snapshots to a JS stream, stopping promptly when the reader cancels.
    pub async fn watch<T: DeserializeOwned + IntoPluginValue + PartialEq + Clone>(
        &self,
        event: &str,
        sink: TypedSink<T>,
        cancel: CancelToken,
    ) -> Result<(), NativePluginError> {
        let mut last = None;
        Self::forward(self.subscribe_latest(event), sink, cancel, move |value| {
            if last.as_ref() == Some(value) {
                return false;
            }
            last = Some(value.clone());
            true
        })
        .await
    }

    /// Forwards every native notification, including equal consecutive payloads.
    pub async fn watch_changes<T: DeserializeOwned + IntoPluginValue>(
        &self,
        event: &str,
        sink: TypedSink<T>,
        cancel: CancelToken,
    ) -> Result<(), NativePluginError> {
        let subscription = async {
            self.event_channel()?
                .subscribe(format!("{}/{event}", self.namespace), "", 16)
                .await
                .map_err(NativePluginError::decode)
        };
        Self::forward(subscription, sink, cancel, |_| true).await
    }

    async fn forward<T: DeserializeOwned + IntoPluginValue>(
        subscription: impl Future<Output = Result<NativeEventStream, NativePluginError>>,
        mut sink: TypedSink<T>,
        cancel: CancelToken,
        mut should_send: impl FnMut(&T) -> bool,
    ) -> Result<(), NativePluginError> {
        let mut forwarding = pin!(async {
            let mut stream = pin!(subscription.await?);
            while let Some(payload) = poll_fn(|cx| stream.as_mut().poll_next(cx)).await {
                let value: T =
                    serde_json::from_str(&payload).map_err(NativePluginError::platform)?;
                if should_send(&value) && sink.send(value).await.is_err() {
                    break;
                }
            }
            Ok(())
        });
        let mut cancelled = pin!(cancel.cancelled());
        poll_fn(|cx| {
            if cancelled.as_mut().poll(cx).is_ready() {
                Poll::Ready(Ok(()))
            } else {
                forwarding.as_mut().poll(cx)
            }
        })
        .await
    }
}
