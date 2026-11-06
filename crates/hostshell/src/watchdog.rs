//! Render-thread stall detector. It never touches the engine or the session.
//! A quiet `inbox.recv` is not a stall: `since_ms == 0` means the thread is idle.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::diagnostics::DiagnosticsHub;

/// How long one command may run after the first successful present.
pub(crate) const STALL_MS: u64 = 5_000;
const TICK: Duration = Duration::from_millis(500);

pub(crate) struct Busy {
    since_ms: AtomicU64,
    tag: Mutex<&'static str>,
    armed: AtomicBool,
    reported: AtomicBool,
}

impl Busy {
    pub(crate) fn new() -> Self {
        Self {
            since_ms: AtomicU64::new(0),
            tag: Mutex::new("idle"),
            armed: AtomicBool::new(false),
            reported: AtomicBool::new(false),
        }
    }

    pub(crate) fn enter(&self, tag: &'static str) {
        *self
            .tag
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = tag;
        self.since_ms.store(
            crate::net::ms_i64(crate::net::mono_ms()) as u64,
            Ordering::Release,
        );
    }

    pub(crate) fn leave(&self) {
        self.since_ms.store(0, Ordering::Release);
    }

    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }
}

/// `since_ms == 0` is idle. The clock starts only after the first present.
pub(crate) fn stalled(armed: bool, since_ms: u64, now_ms: u64) -> bool {
    armed && since_ms != 0 && now_ms.saturating_sub(since_ms) >= STALL_MS
}

pub(crate) fn spawn(busy: Arc<Busy>, hub: Option<Arc<DiagnosticsHub>>) {
    let _ = thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            loop {
                thread::sleep(TICK);
                let since = busy.since_ms.load(Ordering::Acquire);
                let now = crate::net::ms_i64(crate::net::mono_ms()) as u64;
                let armed = busy.armed.load(Ordering::Acquire);
                if !stalled(armed, since, now) {
                    if busy.reported.swap(false, Ordering::AcqRel) {
                        tracing::info!(target: "impasto::watchdog", "render thread recovered");
                    }
                    continue;
                }
                if busy.reported.swap(true, Ordering::AcqRel) {
                    continue;
                }
                let tag = *busy
                    .tag
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let stalled_ms = now.saturating_sub(since);
                tracing::error!(
                    target: "impasto::watchdog",
                    tag,
                    stalled_ms,
                    "render thread stalled"
                );
                if let Some(hub) = hub.as_ref() {
                    hub.error(
                        "watchdog",
                        &format!("render thread stalled in {tag}"),
                        None,
                        None,
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stall_requires_the_clock_and_a_present() {
        assert!(!stalled(false, 1, 10_000));
        assert!(!stalled(true, 0, 10_000));
        assert!(!stalled(true, 1_000, 1_000 + STALL_MS - 1));
        assert!(stalled(true, 1_000, 1_000 + STALL_MS));
    }
}
