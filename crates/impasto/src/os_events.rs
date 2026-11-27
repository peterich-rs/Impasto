//! Fan-out of events from the native plugin attached to a window.

use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

use futures_channel::oneshot;
use futures_core::Stream;

use crate::os_channel::NativeChannel;

const START: &str = "@impasto/events/start";
const STOP: &str = "@impasto/events/stop";

/// Subscriptions and native listeners for one window session.
#[derive(Clone)]
pub struct NativeEventChannel(Arc<Channel>);

struct Channel {
    native: NativeChannel,
    // Serialize start/stop dispatch without blocking event delivery or acknowledgments.
    control: Mutex<()>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    topics: HashMap<String, Topic>,
}

struct Topic {
    generation: u64,
    params: String,
    subscribers: HashMap<u64, Arc<Subscriber>>,
    starting: Option<HashMap<u64, oneshot::Sender<Result<(), String>>>>,
    overflow_reported: bool,
}

struct Subscriber {
    capacity: usize,
    latest: bool,
    buffer: Mutex<Buffer>,
}

#[derive(Default)]
struct Buffer {
    events: VecDeque<String>,
    waker: Option<Waker>,
    closed: bool,
}

/// A bounded stream of raw events from a native plugin.
pub struct NativeEventStream {
    channel: Weak<Channel>,
    topic: String,
    id: u64,
    subscriber: Arc<Subscriber>,
}

impl NativeEventChannel {
    /// Creates per-window subscriptions using the native request channel for control messages.
    #[doc(hidden)]
    pub fn new(native: NativeChannel) -> Self {
        Self(Arc::new(Channel {
            native,
            control: Mutex::default(),
            state: Mutex::default(),
        }))
    }

    /// Subscribes to a discrete event topic and drops the newest event on overflow.
    pub async fn subscribe(
        &self,
        topic: impl Into<String>,
        params: &str,
        capacity: usize,
    ) -> Result<NativeEventStream, String> {
        self.open(topic.into(), params, capacity, false).await
    }

    /// Subscribes to a state topic and replaces the oldest queued state on overflow.
    pub async fn subscribe_latest(
        &self,
        topic: impl Into<String>,
        params: &str,
        capacity: usize,
    ) -> Result<NativeEventStream, String> {
        self.open(topic.into(), params, capacity, true).await
    }

    async fn open(
        &self,
        topic: String,
        params: &str,
        capacity: usize,
        latest: bool,
    ) -> Result<NativeEventStream, String> {
        if topic.is_empty() || topic.chars().any(char::is_control) || capacity == 0 {
            return Err("invalid native event topic or capacity".into());
        }
        let subscriber = Arc::new(Subscriber {
            capacity,
            latest,
            buffer: Mutex::default(),
        });
        let (id, ready) = self.0.register(&topic, params, Arc::clone(&subscriber))?;
        let stream = NativeEventStream {
            channel: Arc::downgrade(&self.0),
            topic,
            id,
            subscriber,
        };
        if let Some(ready) = ready {
            ready
                .await
                .map_err(|_| "native event subscription detached".to_owned())??;
        }
        Ok(stream)
    }

    /// Delivers an event from the platform plugin without waiting for any subscriber.
    #[doc(hidden)]
    pub fn emit(&self, topic: &str, payload: &str) -> bool {
        let mut wakes = Vec::new();
        let overflow = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let Some(topic) = state.topics.get_mut(topic) else {
                return false;
            };
            let mut overflow = false;
            for subscriber in topic.subscribers.values() {
                let mut buffer = subscriber
                    .buffer
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if buffer.events.len() == subscriber.capacity {
                    if !subscriber.latest {
                        overflow = true;
                        continue;
                    }
                    buffer.events.pop_front();
                }
                buffer.events.push_back(payload.to_owned());
                wakes.extend(buffer.waker.take());
            }
            overflow && !std::mem::replace(&mut topic.overflow_reported, true)
        };
        if overflow {
            tracing::warn!(%topic, "native event buffer is full");
        }
        for waker in wakes {
            waker.wake();
        }
        true
    }

    /// Closes subscriptions and asks native plugins to stop their listeners.
    #[doc(hidden)]
    pub fn close(&self) {
        self.retire(true);
    }

    /// Ends existing subscriptions when the session moves to another native window.
    #[doc(hidden)]
    pub fn detach_window(&self) {
        self.retire(false);
    }

    fn retire(&self, close: bool) {
        let control = self
            .0
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let topics = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closed |= close;
            std::mem::take(&mut state.topics)
        };
        for topic in topics.keys() {
            self.0.stop(topic);
        }
        drop(control);
        for (_, topic) in topics {
            topic.close("native event channel detached");
        }
    }
}

type Ready = oneshot::Receiver<Result<(), String>>;

impl Channel {
    fn register(
        self: &Arc<Self>,
        topic: &str,
        params: &str,
        subscriber: Arc<Subscriber>,
    ) -> Result<(u64, Option<Ready>), String> {
        let control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (id, ready, start) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.closed {
                return Err("native event channel detached".into());
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or("native event subscriber identifiers exhausted")?;
            let id = state.next;
            let entry = state
                .topics
                .entry(topic.to_owned())
                .or_insert_with(|| Topic {
                    generation: id,
                    params: params.to_owned(),
                    subscribers: HashMap::new(),
                    starting: Some(HashMap::new()),
                    overflow_reported: false,
                });
            if entry.params != params {
                return Err(format!(
                    "native event topic {topic} is in use with different parameters"
                ));
            }
            entry.subscribers.insert(id, subscriber);
            let ready = entry.starting.as_mut().map(|waiters| {
                let (ready, receiver) = oneshot::channel();
                waiters.insert(id, ready);
                receiver
            });
            (id, ready, entry.generation == id)
        };
        if start {
            let weak = Arc::downgrade(self);
            let name = topic.to_owned();
            let payload = serde_json::json!({ "topic": topic, "params": params }).to_string();
            let sent = self.native.notify(
                START,
                &payload,
                Box::new(move |reply| {
                    if let Some(channel) = weak.upgrade() {
                        channel.started(&name, id, reply.map(|_| ()));
                    }
                }),
            );
            if let Err(error) = sent {
                drop(control);
                self.started(topic, id, Err(error));
            }
        }
        Ok((id, ready))
    }

    fn started(&self, name: &str, generation: u64, result: Result<(), String>) {
        let (waiters, removed, idle) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(topic) = state
                .topics
                .get_mut(name)
                .filter(|topic| topic.generation == generation)
            else {
                return;
            };
            let waiters = topic.starting.take().unwrap_or_default();
            let idle = topic.subscribers.is_empty();
            let removed = if result.is_err() {
                state.topics.remove(name)
            } else {
                None
            };
            (waiters, removed, idle)
        };
        if let Some(topic) = removed {
            topic.close("native event listener could not start");
        }
        for waiter in waiters.into_values() {
            let _ = waiter.send(result.clone());
        }
        if idle && result.is_ok() {
            self.stop_if_unused(name, Some(generation));
        }
    }

    fn stop(&self, topic: &str) {
        let payload = serde_json::json!({ "topic": topic }).to_string();
        let _ = self.native.notify(STOP, &payload, Box::new(|_| {}));
    }

    fn stop_if_unused(&self, topic: &str, generation: Option<u64>) {
        let _control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let stopped = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.topics.get(topic).is_some_and(|entry| {
                entry.subscribers.is_empty()
                    && entry.starting.is_none()
                    && generation.is_none_or(|generation| generation == entry.generation)
            }) {
                state.topics.remove(topic);
                true
            } else {
                false
            }
        };
        if stopped {
            self.stop(topic);
        }
    }

    fn unsubscribe(&self, topic: &str, id: u64) {
        let (idle, pending) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(entry) = state.topics.get_mut(topic) else {
                return;
            };
            entry.subscribers.remove(&id);
            let pending = entry
                .starting
                .as_mut()
                .and_then(|waiters| waiters.remove(&id));
            (
                entry.subscribers.is_empty() && entry.starting.is_none(),
                pending,
            )
        };
        drop(pending);
        if idle {
            self.stop_if_unused(topic, None);
        }
    }
}

impl Topic {
    fn close(self, error: &str) {
        for waiter in self.starting.into_iter().flat_map(HashMap::into_values) {
            let _ = waiter.send(Err(error.to_owned()));
        }
        for subscriber in self.subscribers.into_values() {
            let waker = {
                let mut buffer = subscriber
                    .buffer
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                buffer.closed = true;
                buffer.events.clear();
                buffer.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl Stream for NativeEventStream {
    type Item = String;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut buffer = self
            .subscriber
            .buffer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(event) = buffer.events.pop_front() {
            Poll::Ready(Some(event))
        } else if buffer.closed {
            Poll::Ready(None)
        } else {
            buffer.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for NativeEventStream {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.upgrade() {
            channel.unsubscribe(&self.topic, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    #[test]
    fn dropped_pending_subscription_stops_after_native_start_acknowledgment() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let native = NativeChannel::new({
            let sent = Arc::clone(&sent);
            move |id, operation, _| {
                sent.lock().unwrap().push((id, operation.to_owned()));
                true
            }
        });
        let channel = NativeEventChannel::new(native.clone());
        let mut cx = Context::from_waker(Waker::noop());
        let mut pending = Box::pin(channel.subscribe("test/changes", "{}", 1));
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        drop(pending);
        let start = {
            let sent = sent.lock().unwrap();
            assert_eq!(sent.len(), 1);
            sent[0].0
        };
        native.complete(start, Ok(None));
        assert_eq!(sent.lock().unwrap()[1].1, STOP);
    }

    #[test]
    fn native_listeners_are_shared_and_backpressure_preserves_the_requested_events() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let native = Arc::new_cyclic(|channel: &Weak<NativeChannel>| {
            let channel = Weak::clone(channel);
            let sent = Arc::clone(&sent);
            NativeChannel::new(move |id, operation, _| {
                sent.lock().unwrap().push(operation.to_owned());
                channel.upgrade().unwrap().complete(id, Ok(None))
            })
        });
        let channel = NativeEventChannel::new((*native).clone());
        let mut cx = Context::from_waker(Waker::noop());
        let newest =
            std::pin::pin!(channel.subscribe_latest("test/changes", "{}", 1)).poll(&mut cx);
        let oldest = std::pin::pin!(channel.subscribe("test/changes", "{}", 1)).poll(&mut cx);
        let (Poll::Ready(Ok(mut newest)), Poll::Ready(Ok(mut oldest))) = (newest, oldest) else {
            panic!("the simulated listener did not acknowledge its start");
        };
        assert!(channel.emit("test/changes", "first"));
        assert!(channel.emit("test/changes", "second"));
        assert_eq!(
            Pin::new(&mut newest).poll_next(&mut cx),
            Poll::Ready(Some("second".into()))
        );
        assert_eq!(
            Pin::new(&mut oldest).poll_next(&mut cx),
            Poll::Ready(Some("first".into()))
        );
        drop(newest);
        assert_eq!(*sent.lock().unwrap(), [START]);
        channel.close();
        assert_eq!(*sent.lock().unwrap(), [START, STOP]);
        assert_eq!(Pin::new(&mut oldest).poll_next(&mut cx), Poll::Ready(None));
    }
}
