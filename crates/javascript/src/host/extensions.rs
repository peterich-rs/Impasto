//! Values and cancellation shared by native extensions and the transport bridge.

use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::thread::{self, ThreadId};
use std::time::Duration;

use super::io::{Waiter, Waiters};
use super::{CreditPool, Delivery, IoEvent, PluginValue, Route, TERMINAL_PAYLOAD_LIMIT};

#[derive(Debug, Default)]
struct CancelInner {
    flag: Arc<AtomicBool>,
    wakers: Mutex<Waiters>,
    pools: Option<(Arc<CreditPool>, Arc<CreditPool>)>,
}

#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    inner: Arc<CancelInner>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn routed(
        flag: Arc<AtomicBool>,
        ingress: Arc<CreditPool>,
        outbound: Arc<CreditPool>,
    ) -> Self {
        Self {
            inner: Arc::new(CancelInner {
                flag,
                wakers: Mutex::default(),
                pools: Some((ingress, outbound)),
            }),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::Release);
        let wakers = {
            let mut guard = self
                .inner
                .wakers
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::take(&mut *guard)
        };
        wakers.wake_all();
        if let Some((ingress, outbound)) = &self.inner.pools {
            ingress.notify_cancel();
            outbound.notify_cancel();
        }
    }

    pub async fn cancelled(&self) {
        let waiter = Arc::new(Waiter::default());
        poll_fn(|cx| {
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            if let Some((ingress, _)) = &self.inner.pools
                && ingress.poll_closed(&waiter, cx.waker()).is_ready()
            {
                return Poll::Ready(());
            }
            let mut wakers = self
                .inner
                .wakers
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            wakers.register(&waiter, cx.waker());
            Poll::Pending
        })
        .await;
    }

    pub(super) fn flag(&self) -> &AtomicBool {
        &self.inner.flag
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
    deliver: Arc<dyn Fn(Delivery) + Send + Sync>,
    finish_on_drop: bool,
    ended: bool,
    consumer: ThreadId,
}

impl StreamSink {
    #[cfg(feature = "runtime")]
    pub(crate) fn new(route: Route, deliver: impl Fn(Delivery) + Send + Sync + 'static) -> Self {
        Self {
            route,
            deliver: Arc::new(deliver),
            finish_on_drop: true,
            ended: false,
            consumer: thread::current().id(),
        }
    }

    pub fn cancellation(&self) -> CancelToken {
        self.route.cancellation()
    }

    /// Retain terminal delivery while moving the producer into an async call.
    pub fn terminal_guard(&mut self) -> Self {
        self.finish_on_drop = false;
        Self {
            route: self.route.clone(),
            deliver: Arc::clone(&self.deliver),
            ended: false,
            consumer: self.consumer,
            finish_on_drop: true,
        }
    }

    pub fn max_piece_bytes(&self) -> usize {
        self.route.max_piece_bytes()
    }

    fn prepare_send(&self, item: &PluginValue) -> Result<usize, StreamSendError> {
        if !self.route.is_active() || self.ended {
            return Err(StreamSendError::Closed);
        }
        let bytes = item.payload_bytes();
        if bytes > self.route.max_piece_bytes() {
            return Err(StreamSendError::TooLarge);
        }
        Ok(bytes)
    }

    pub fn try_send(&mut self, item: PluginValue) -> Result<(), StreamSendError> {
        self.send(item, Duration::ZERO)
    }

    pub fn send(&mut self, item: PluginValue, timeout: Duration) -> Result<(), StreamSendError> {
        let bytes = self.prepare_send(&item)?;
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

    pub async fn send_async(&mut self, item: PluginValue) -> Result<(), StreamSendError> {
        let bytes = self.prepare_send(&item)?;
        if thread::current().id() == self.consumer {
            return self.send(item, Duration::ZERO);
        }
        let Some(credit) = self.route.reserve_async(bytes).await else {
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
        if self.finish_on_drop {
            self.terminal(StreamEvent::Finished);
        }
    }
}

/// A thread-affine native resource. Drop releases local state. cancel also
/// signals any work the resource started with the supplied token.
pub trait NativeResource: 'static {
    /// Named JS methods supplied by a typed object. Legacy resources use call().
    fn methods(&self) -> &'static [&'static str] {
        &[]
    }

    /// Prepare an owned call while the resource is borrowed on its owner.
    fn method(&mut self, _name: &str, _cancel: &CancelToken) -> Option<super::Method> {
        None
    }

    fn call(
        &mut self,
        method: &str,
        args: &[PluginValue],
        cancel: &CancelToken,
    ) -> Result<PluginValue, String>;
}
