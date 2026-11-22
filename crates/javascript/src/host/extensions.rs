//! Values and cancellation shared by native extensions and the transport bridge.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, ThreadId};
use std::time::Duration;

use super::{CreditPool, Delivery, IoEvent, PluginValue, Route, TERMINAL_PAYLOAD_LIMIT};

#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    pools: Option<(Arc<CreditPool>, Arc<CreditPool>)>,
}

impl CancelToken {
    pub(super) fn routed(
        flag: Arc<AtomicBool>,
        ingress: Arc<CreditPool>,
        outbound: Arc<CreditPool>,
    ) -> Self {
        Self {
            flag,
            pools: Some((ingress, outbound)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        if let Some((ingress, outbound)) = &self.pools {
            ingress.notify_cancel();
            outbound.notify_cancel();
        }
    }

    pub(super) fn flag(&self) -> &AtomicBool {
        &self.flag
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSendError {
    Closed,
    TooLarge,
    Backpressure,
}

impl std::fmt::Display for StreamSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Closed => "stream is closed",
            Self::TooLarge => "stream item is larger than the credit limit",
            Self::Backpressure => "stream has no credit available",
        })
    }
}

impl std::error::Error for StreamSendError {}

#[derive(Debug)]
pub enum StreamEvent {
    Item(PluginValue),
    Finished,
    Failed(String),
}

/// A single producer. Credit travels with each item until the JavaScript
/// reader consumes or drops it. Dropping the producer closes the stream.
pub struct StreamSink {
    route: Route,
    deliver: Box<dyn Fn(Delivery) + Send + Sync>,
    ended: bool,
    consumer: ThreadId,
}

impl StreamSink {
    #[cfg(feature = "runtime")]
    pub(crate) fn new(route: Route, deliver: impl Fn(Delivery) + Send + Sync + 'static) -> Self {
        Self {
            route,
            deliver: Box::new(deliver),
            ended: false,
            consumer: thread::current().id(),
        }
    }

    pub fn cancellation(&self) -> CancelToken {
        self.route.cancellation()
    }

    pub fn send(&mut self, item: PluginValue, timeout: Duration) -> Result<(), StreamSendError> {
        if !self.route.is_active() || self.ended {
            return Err(StreamSendError::Closed);
        }
        let bytes = item.payload_bytes();
        if bytes > self.route.max_piece_bytes() {
            return Err(StreamSendError::TooLarge);
        }
        let timeout = if thread::current().id() == self.consumer {
            Duration::ZERO
        } else {
            timeout
        };
        let Some(credit) = self.route.reserve(bytes, timeout) else {
            return Err(if self.route.is_active() {
                StreamSendError::Backpressure
            } else {
                StreamSendError::Closed
            });
        };
        if !self.route.is_active() {
            return Err(StreamSendError::Closed);
        }
        self.deliver(StreamEvent::Item(item), Some(credit));
        Ok(())
    }

    pub fn finish(mut self) {
        self.terminal(StreamEvent::Finished);
    }

    pub fn fail(mut self, mut message: String) {
        let end = message.floor_char_boundary(TERMINAL_PAYLOAD_LIMIT);
        message.truncate(end);
        self.terminal(StreamEvent::Failed(message));
    }

    fn terminal(&mut self, event: StreamEvent) {
        if !std::mem::replace(&mut self.ended, true) && self.route.claim_terminal() {
            self.deliver(event, None);
        }
    }

    fn deliver(&self, event: StreamEvent, credit: Option<super::Credit>) {
        let event = IoEvent::NativeStream {
            id: self.route.key.sequence,
            event,
        };
        (self.deliver)(self.route.deliver(event, credit));
    }
}

impl Drop for StreamSink {
    fn drop(&mut self) {
        self.terminal(StreamEvent::Finished);
    }
}

/// A thread-affine native resource. Drop releases local state. cancel also
/// signals any work the resource started with the supplied token.
pub trait NativeResource: 'static {
    fn call(
        &mut self,
        method: &str,
        args: &[PluginValue],
        cancel: &CancelToken,
    ) -> Result<PluginValue, String>;
}
