use std::collections::HashMap;
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use super::IoEvent;
use crate::host::CancelToken;

pub(in crate::host) fn register_waker(wakers: &mut Vec<Waker>, waker: &Waker) {
    if !wakers.iter().any(|existing| existing.will_wake(waker)) {
        wakers.push(waker.clone());
    }
}

pub(in crate::host) fn wake_all(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

/// A single cancellation future owns its waker. Its queues keep weak
/// registrations so dropping a completed future also releases its task.
#[derive(Debug, Default)]
pub(in crate::host) struct Waiter(Mutex<Option<Waker>>);

#[derive(Debug, Default)]
pub(in crate::host) struct Waiters(Vec<Weak<Waiter>>);

impl Waiters {
    pub(in crate::host) fn register(&mut self, waiter: &Arc<Waiter>, waker: &Waker) {
        let weak = Arc::downgrade(waiter);
        self.0.retain(|entry| entry.strong_count() > 0);
        if !self.0.iter().any(|entry| Weak::ptr_eq(entry, &weak)) {
            self.0.push(weak);
        }
        let mut slot = waiter.0.lock().unwrap_or_else(|error| error.into_inner());
        match slot.as_mut() {
            Some(saved) => saved.clone_from(waker),
            None => *slot = Some(waker.clone()),
        }
    }

    pub(in crate::host) fn wake_all(self) {
        for waiter in self.0.into_iter().filter_map(|waiter| waiter.upgrade()) {
            let waker = waiter
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

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
    wakers: Vec<Waker>,
    close_wakers: Waiters,
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
                wakers: Vec::new(),
                close_wakers: Waiters::default(),
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
        let mut state = self.lock();
        state.open = false;
        let wakers = std::mem::take(&mut state.wakers);
        let close_wakers = std::mem::take(&mut state.close_wakers);
        drop(state);
        self.changed.notify_all();
        wake_all(wakers);
        close_wakers.wake_all();
    }

    pub fn notify_cancel(&self) {
        // Synchronize with reserve between its cancellation test and wait.
        let mut state = self.lock();
        let wakers = std::mem::take(&mut state.wakers);
        drop(state);
        self.changed.notify_all();
        wake_all(wakers);
    }

    pub fn is_open(&self) -> bool {
        self.lock().open
    }

    pub(in crate::host) fn poll_closed(&self, waiter: &Arc<Waiter>, waker: &Waker) -> Poll<()> {
        let mut state = self.lock();
        if !state.open {
            return Poll::Ready(());
        }
        state.close_wakers.register(waiter, waker);
        Poll::Pending
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

    fn try_reserve_locked(
        self: &Arc<Self>,
        state: &mut State,
        sequence: u64,
        bytes: usize,
        max_resource_bytes: usize,
    ) -> Option<Credit> {
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
            Some(Credit {
                pool: Arc::clone(self),
                sequence,
                bytes,
            })
        } else {
            None
        }
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
            if let Some(credit) =
                self.try_reserve_locked(&mut state, sequence, bytes, max_resource_bytes)
            {
                return Some(credit);
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

    fn poll_reserve(
        self: &Arc<Self>,
        sequence: u64,
        bytes: usize,
        max_resource_bytes: usize,
        cancel: &AtomicBool,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Credit>> {
        let max_resource_bytes = max_resource_bytes.min(self.bytes);
        if bytes > max_resource_bytes {
            return Poll::Ready(None);
        }
        let mut state = self.lock();
        if !state.open || cancel.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        if let Some(credit) =
            self.try_reserve_locked(&mut state, sequence, bytes, max_resource_bytes)
        {
            return Poll::Ready(Some(credit));
        }
        register_waker(&mut state.wakers, cx.waker());
        Poll::Pending
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
        let wakers = std::mem::take(&mut state.wakers);
        drop(state);
        self.pool.changed.notify_all();
        wake_all(wakers);
    }
}

/// Scoped completion and queue limits for one request on a background thread.
#[derive(Clone, Debug)]
pub struct Route {
    pub key: RequestKey,
    ingress: Arc<CreditPool>,
    outbound: Arc<CreditPool>,
    canceled: CancelToken,
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
        let canceled = CancelToken::routed(canceled, Arc::clone(&ingress), Arc::clone(&outbound));
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
        !self.is_cancelled() && self.ingress.is_open()
    }

    pub fn is_cancelled(&self) -> bool {
        self.canceled.is_cancelled()
    }

    pub fn cancel(&self) {
        self.canceled.cancel();
    }

    pub fn cancellation(&self) -> CancelToken {
        self.canceled.clone()
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
            self.canceled.flag(),
            timeout,
        )
    }

    pub fn poll_reserve(&self, bytes: usize, cx: &mut Context<'_>) -> Poll<Option<Credit>> {
        self.ingress.poll_reserve(
            self.key.sequence,
            bytes,
            self.max_piece_bytes(),
            self.canceled.flag(),
            cx,
        )
    }

    pub async fn reserve_async(&self, bytes: usize) -> Option<Credit> {
        poll_fn(|cx| self.poll_reserve(bytes, cx)).await
    }

    pub fn reserve_upload(&self, bytes: usize) -> Option<Credit> {
        self.outbound.reserve(
            self.key.sequence,
            bytes,
            self.outbound.bytes,
            self.canceled.flag(),
            Duration::ZERO,
        )
    }

    pub fn reserve_send(&self) -> Option<Credit> {
        self.outbound.reserve(
            self.key.sequence,
            self.send_bytes,
            self.outbound.resource_bytes,
            self.canceled.flag(),
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

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    use super::*;

    struct CountWake(AtomicUsize);

    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn route(pool: &Arc<CreditPool>) -> Route {
        Route::new(
            RequestKey {
                runtime: RuntimeKey {
                    instance: 1,
                    generation: 1,
                },
                sequence: 1,
            },
            Arc::clone(pool),
            Arc::clone(pool),
            Arc::new(AtomicBool::new(false)),
            true,
        )
    }

    #[test]
    fn poll_reserve_wakes_on_credit_drop_and_deduplicates_wakers() {
        let pool = Arc::new(CreditPool::new(8, 1, 8, 1));
        let r = route(&pool);
        let first = r.reserve(4, Duration::ZERO).expect("first credit");

        let wake = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(r.reserve_async(4));

        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert_eq!(pool.lock().wakers.len(), 1);

        drop(first);
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);

        let Poll::Ready(Some(second)) = fut.as_mut().poll(&mut cx) else {
            panic!("expected ready credit after release");
        };
        drop(second);
    }

    #[test]
    fn cancel_token_cancelled_future_wakes_on_cancel_and_pool_close() {
        let standalone = CancelToken::new();
        let wake = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wake));
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(standalone.cancelled());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        standalone.cancel();
        assert_eq!(wake.0.load(Ordering::SeqCst), 1);
        assert!(fut.as_mut().poll(&mut cx).is_ready());

        let pool = Arc::new(CreditPool::new(8, 1, 8, 1));
        let r = route(&pool);
        let routed = r.cancellation();
        let wake2 = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker2 = Waker::from(Arc::clone(&wake2));
        let mut cx2 = Context::from_waker(&waker2);
        let mut fut2 = pin!(routed.cancelled());
        assert!(fut2.as_mut().poll(&mut cx2).is_pending());
        pool.close();
        assert_eq!(wake2.0.load(Ordering::SeqCst), 1);
        assert!(fut2.as_mut().poll(&mut cx2).is_ready());
    }
}
