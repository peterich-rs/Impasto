use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::IoEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeKey {
    pub instance: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestKey {
    pub runtime: RuntimeKey,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreditUsage {
    pub events: usize,
    pub bytes: usize,
    pub peak_events: usize,
    pub peak_bytes: usize,
}

#[derive(Debug)]
struct State {
    open: bool,
    usage: CreditUsage,
    resources: HashMap<u64, (usize, usize)>,
}

#[derive(Debug)]
pub struct CreditPool {
    state: Mutex<State>,
    changed: Condvar,
    bytes: usize,
    events: usize,
    resource_bytes: usize,
    resource_events: usize,
}

impl CreditPool {
    pub fn new(bytes: usize, events: usize, resource_bytes: usize, resource_events: usize) -> Self {
        Self {
            state: Mutex::new(State {
                open: true,
                usage: CreditUsage::default(),
                resources: HashMap::new(),
            }),
            changed: Condvar::new(),
            bytes,
            events,
            resource_bytes,
            resource_events,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn close(&self) {
        self.lock().open = false;
        self.changed.notify_all();
    }

    pub fn notify_cancel(&self) {
        // Synchronize with reserve between its cancellation test and wait.
        drop(self.lock());
        self.changed.notify_all();
    }

    pub fn is_open(&self) -> bool {
        self.lock().open
    }

    pub fn usage(&self) -> CreditUsage {
        self.lock().usage
    }

    pub fn bytes_for(&self, sequence: u64) -> usize {
        self.lock()
            .resources
            .get(&sequence)
            .map_or(0, |usage| usage.1)
    }

    fn reserve(
        self: &Arc<Self>,
        sequence: u64,
        bytes: usize,
        max_resource_bytes: usize,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Option<Credit> {
        let max_resource_bytes = max_resource_bytes.min(self.bytes);
        if bytes > max_resource_bytes {
            return None;
        }
        let start = Instant::now();
        let mut state = self.lock();
        loop {
            if !state.open || cancel.load(Ordering::Acquire) {
                return None;
            }
            let (count, used) = state.resources.get(&sequence).copied().unwrap_or_default();
            if state.usage.events < self.events
                && bytes <= self.bytes - state.usage.bytes
                && count < self.resource_events
                && bytes <= max_resource_bytes.saturating_sub(used)
            {
                state.resources.insert(sequence, (count + 1, used + bytes));
                state.usage.events += 1;
                state.usage.bytes += bytes;
                state.usage.peak_bytes = state.usage.peak_bytes.max(state.usage.bytes);
                state.usage.peak_events = state.usage.peak_events.max(state.usage.events);
                return Some(Credit {
                    pool: Arc::clone(self),
                    sequence,
                    bytes,
                });
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return None;
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}

/// A payload reservation that follows the event to the consumer.
#[derive(Debug)]
pub struct Credit {
    pool: Arc<CreditPool>,
    sequence: u64,
    bytes: usize,
}

impl Credit {
    pub fn covers(&self, pool: &Arc<CreditPool>, sequence: u64, bytes: usize) -> bool {
        Arc::ptr_eq(&self.pool, pool) && self.sequence == sequence && self.bytes >= bytes
    }
}

impl Drop for Credit {
    fn drop(&mut self) {
        let mut state = self.pool.lock();
        state.usage.bytes -= self.bytes;
        state.usage.events -= 1;
        if let Some((count, used)) = state.resources.get_mut(&self.sequence) {
            *count -= 1;
            *used -= self.bytes;
            if *count == 0 {
                state.resources.remove(&self.sequence);
            }
        }
        drop(state);
        self.pool.changed.notify_all();
    }
}

/// Scoped completion and queue limits for one request on a background thread.
#[derive(Clone, Debug)]
pub struct Route {
    pub key: RequestKey,
    ingress: Arc<CreditPool>,
    outbound: Arc<CreditPool>,
    canceled: Arc<AtomicBool>,
    terminal: Arc<AtomicBool>,
    stream: bool,
    send_bytes: usize,
}

impl Route {
    pub fn new(
        key: RequestKey,
        ingress: Arc<CreditPool>,
        outbound: Arc<CreditPool>,
        canceled: Arc<AtomicBool>,
        stream: bool,
    ) -> Self {
        Self {
            key,
            ingress,
            outbound,
            canceled,
            terminal: Arc::new(AtomicBool::new(false)),
            stream,
            send_bytes: 0,
        }
    }

    pub fn with_send_bytes(mut self, bytes: usize) -> Self {
        self.send_bytes = bytes;
        self
    }

    pub fn buffered_amount(&self) -> usize {
        self.outbound.bytes_for(self.key.sequence)
    }

    pub fn is_active(&self) -> bool {
        !self.canceled.load(Ordering::Acquire) && self.ingress.is_open()
    }

    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        self.ingress.notify_cancel();
        self.outbound.notify_cancel();
    }

    pub fn claim_terminal(&self) -> bool {
        self.is_active() && !self.terminal.swap(true, Ordering::AcqRel)
    }

    pub fn max_piece_bytes(&self) -> usize {
        if self.stream {
            self.ingress.resource_bytes
        } else {
            self.ingress.bytes
        }
    }

    pub fn reserve(&self, bytes: usize, timeout: Duration) -> Option<Credit> {
        self.ingress.reserve(
            self.key.sequence,
            bytes,
            self.max_piece_bytes(),
            &self.canceled,
            timeout,
        )
    }

    pub fn reserve_upload(&self, bytes: usize) -> Option<Credit> {
        self.outbound.reserve(
            self.key.sequence,
            bytes,
            self.outbound.bytes,
            &self.canceled,
            Duration::ZERO,
        )
    }

    pub fn reserve_send(&self) -> Option<Credit> {
        self.outbound.reserve(
            self.key.sequence,
            self.send_bytes,
            self.outbound.resource_bytes,
            &self.canceled,
            Duration::ZERO,
        )
    }

    pub fn deliver(&self, event: IoEvent, credit: Option<Credit>) -> Delivery {
        Delivery {
            runtime: self.key.runtime,
            event,
            credit,
        }
    }
}

pub struct Delivery {
    pub runtime: RuntimeKey,
    pub event: IoEvent,
    pub credit: Option<Credit>,
}
