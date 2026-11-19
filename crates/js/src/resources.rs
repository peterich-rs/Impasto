use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use js_host::{Delivery, HttpError, HttpFailure, IoEvent, MAX_SAFE_ID, Route, RuntimeKey};

/// Allocate a safe integer for each compatibility token. A naked late token
/// cannot settle an operation in a newer instance, even through an old sink.
pub(crate) fn next_id() -> Option<u64> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        (id <= MAX_SAFE_ID).then(|| id + 1)
    })
    .ok()
}

#[expect(
    clippy::cast_precision_loss,
    reason = "IDs never exceed Number.MAX_SAFE_INTEGER"
)]
pub(crate) fn number(id: u64) -> f64 {
    id as f64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SocketState {
    Connecting,
    Open,
    Closing,
}

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Http,
    Stream { head: bool },
    Plugin,
    Socket(SocketState),
}

pub(crate) struct Resource {
    pub kind: Kind,
    pub deadline: Option<i64>,
    pub route: Route,
}

#[derive(Default)]
pub(crate) struct Resources {
    entries: HashMap<u64, Resource>,
    pub peak: usize,
    pub socket_peak: usize,
    pub discarded: usize,
}

impl Resources {
    pub fn get(&self, id: u64) -> Option<&Resource> {
        self.entries.get(&id)
    }

    pub fn insert(&mut self, id: u64, resource: Resource) {
        self.entries.insert(id, resource);
        self.peak = self.peak.max(self.requests());
        self.socket_peak = self.socket_peak.max(self.sockets());
    }

    pub fn remove(&mut self, id: u64) -> Option<Resource> {
        self.entries.remove(&id)
    }

    pub fn set_kind(&mut self, id: u64, kind: Kind, deadline: Option<i64>) {
        if let Some(resource) = self.entries.get_mut(&id) {
            resource.kind = kind;
            resource.deadline = deadline;
        }
    }

    pub fn requests(&self) -> usize {
        self.entries.len() - self.sockets()
    }

    pub fn sockets(&self) -> usize {
        self.entries
            .values()
            .filter(|resource| matches!(resource.kind, Kind::Socket(_)))
            .count()
    }

    pub fn deadline(&self) -> Option<i64> {
        self.entries
            .values()
            .filter_map(|resource| resource.deadline)
            .min()
    }

    pub fn expired(&self, now: i64) -> Vec<u64> {
        self.entries
            .iter()
            .filter_map(|(id, resource)| {
                resource
                    .deadline
                    .filter(|deadline| *deadline <= now)
                    .map(|_| *id)
            })
            .collect()
    }

    pub fn shift_deadlines(&mut self, offset: i64) {
        for resource in self.entries.values_mut() {
            resource.deadline = resource
                .deadline
                .map(|deadline| deadline.saturating_add(offset));
        }
    }

    pub fn close(&mut self) {
        for resource in self.entries.drain().map(|(_, resource)| resource) {
            resource.route.cancel();
        }
    }

    pub fn accepts(&mut self, event: &IoEvent, local: bool) -> bool {
        let Some(id) = event.id() else { return true };
        let Some(resource) = self.entries.get(&id) else {
            self.discarded = self.discarded.saturating_add(1);
            return false;
        };
        if !local && !resource.route.is_active() {
            self.discarded = self.discarded.saturating_add(1);
            return false;
        }
        let (valid, next) = match (resource.kind, event) {
            (Kind::Http, IoEvent::Http { .. }) | (Kind::Plugin, IoEvent::Plugin { .. }) => {
                (true, None)
            }
            (Kind::Stream { head: false }, IoEvent::HttpHead { .. }) => {
                (true, Some(Kind::Stream { head: true }))
            }
            (Kind::Stream { head }, IoEvent::HttpChunk { done, error, .. }) => (
                head || *done || error.is_some(),
                (!done && error.is_none()).then_some(resource.kind),
            ),
            (Kind::Socket(state), IoEvent::Socket { event, .. }) => match event.as_str() {
                "open" if state == SocketState::Connecting => {
                    (true, Some(Kind::Socket(SocketState::Open)))
                }
                "message" if state == SocketState::Open => (true, Some(resource.kind)),
                "close" | "error" => (true, None),
                _ => (false, None),
            },
            _ => (false, None),
        };
        if valid {
            if let Some(kind) = next {
                let deadline = if matches!(kind, Kind::Socket(SocketState::Open)) {
                    None
                } else {
                    resource.deadline
                };
                self.set_kind(id, kind, deadline);
            } else if let Some(resource) = self.remove(id) {
                resource.route.cancel();
            }
        } else {
            self.discarded = self.discarded.saturating_add(1);
        }
        valid
    }
}

pub(crate) fn failure(id: u64, kind: Kind, failure: HttpFailure, message: &str) -> IoEvent {
    let error = HttpError::new(failure, message);
    match kind {
        Kind::Http => IoEvent::Http {
            token: id,
            result: Err(error),
        },
        Kind::Stream { .. } => IoEvent::HttpChunk {
            token: id,
            bytes: Vec::new(),
            done: true,
            error: Some(error),
        },
        Kind::Socket(_) => IoEvent::Socket {
            id,
            event: "error".into(),
            text: true,
            data: message.as_bytes().to_vec(),
        },
        Kind::Plugin => IoEvent::Plugin {
            token: id,
            result: js_host::PluginResult::Failed {
                message: message.into(),
            },
        },
    }
}

pub type Wake = Arc<dyn Fn() + Send + Sync>;

struct Queue {
    events: VecDeque<Delivery>,
    open: bool,
}

pub(crate) struct Inbox {
    key: RuntimeKey,
    queue: Mutex<Queue>,
    wake: Option<Wake>,
}

impl Inbox {
    pub fn new(key: RuntimeKey, wake: Option<Wake>) -> Self {
        Self {
            key,
            queue: Mutex::new(Queue {
                events: VecDeque::new(),
                open: true,
            }),
            wake,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn push(&self, delivery: Delivery) -> Result<(), Delivery> {
        let mut queue = self.lock();
        if !queue.open || delivery.runtime != self.key {
            return Err(delivery);
        }
        let wake = queue.events.is_empty();
        queue.events.push_back(delivery);
        drop(queue);
        if wake && let Some(wake) = &self.wake {
            wake();
        }
        Ok(())
    }

    pub fn pop(&self) -> Option<Delivery> {
        self.lock().events.pop_front()
    }

    pub fn len(&self) -> usize {
        self.lock().events.len()
    }

    pub fn discard(&self, id: u64) {
        self.lock()
            .events
            .retain(|delivery| delivery.event.id() != Some(id));
    }

    pub fn close(&self) {
        let mut queue = self.lock();
        queue.open = false;
        queue.events.clear();
    }
}
