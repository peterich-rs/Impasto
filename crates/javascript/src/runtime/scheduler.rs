use std::cell::{Cell, RefCell};

use rquickjs::function::Args;
use rquickjs::{Ctx, Function};

use crate::runtime::quickjs::{Saved, Timer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrivePolicy {
    #[default]
    LegacyStrict,
    Yielding,
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeLimits {
    pub heap_bytes: usize,
    pub stack_bytes: usize,
    pub eval_budget_ms: i64,
    pub callback_budget_ms: i64,
    pub timer_batch: u32,
    pub job_batch: u32,
    pub event_batch: u32,
    pub timers: usize,
    pub tickets: usize,
    pub listeners: usize,
    pub requests: usize,
    pub queue_events: usize,
    pub queue_bytes: usize,
    pub stream_events: usize,
    pub stream_bytes: usize,
    pub request_timeout_ms: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            heap_bytes: 16 << 20,
            stack_bytes: 8 << 20,
            eval_budget_ms: 1_000,
            callback_budget_ms: 128,
            timer_batch: 64,
            job_batch: 32,
            event_batch: 128,
            timers: 1_024,
            tickets: 64,
            listeners: 1_024,
            requests: 32,
            queue_events: 256,
            queue_bytes: 16 << 20,
            stream_events: 64,
            stream_bytes: 256 << 10,
            request_timeout_ms: 30_000,
        }
    }
}

impl RuntimeLimits {
    pub(crate) fn valid(self) -> bool {
        self.heap_bytes > 0
            && self.stack_bytes > 0
            && self.eval_budget_ms > 0
            && self.callback_budget_ms > 0
            && self.timer_batch > 0
            && self.job_batch > 0
            && self.event_batch > 0
            && self.queue_events > 0
            && self.stream_events > 0
            && self.stream_bytes > 0
            && self.stream_bytes <= self.queue_bytes
            && self.request_timeout_ms > 0
    }
}

#[derive(Default)]
pub(crate) struct Scheduler {
    pub timers: RefCell<Vec<Timer>>,
    next_id: Cell<u64>,
    pub peak: Cell<usize>,
    pub turn: Cell<u64>,
    pub mono: Cell<i64>,
    origin: Cell<Option<i64>>,
}

impl Scheduler {
    pub fn allocate_id(&self) -> Option<u64> {
        let id = self
            .next_id
            .get()
            .checked_add(1)
            .filter(|id| *id <= crate::host::MAX_SAFE_ID)?;
        self.next_id.set(id);
        Some(id)
    }

    pub fn set_clock(&self, now_ms: i64) -> bool {
        self.mono.set(now_ms);
        let first = self.origin.get().is_none();
        if first {
            self.origin.set(Some(now_ms));
        }
        first
    }

    pub fn now(&self) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "performance.now returns a JavaScript Number"
        )]
        {
            self.mono
                .get()
                .saturating_sub(self.origin.get().unwrap_or(0)) as f64
        }
    }

    pub fn begin_turn(&self) {
        self.turn.set(self.turn.get().saturating_add(1));
    }

    pub fn next_timer_ms(&self) -> Option<i64> {
        self.timers
            .borrow()
            .iter()
            .filter(|timer| timer.active)
            .map(|timer| timer.due)
            .min()
    }

    pub fn call_timer<'js>(
        ctx: &Ctx<'js>,
        (callback, extra): (Saved, Vec<Saved>),
    ) -> rquickjs::Result<()> {
        let function = Function::from_value(callback.restore(ctx)?)?;
        let mut args = Args::new(ctx.clone(), extra.len());
        for value in extra {
            args.push_arg(value.restore(ctx)?)?;
        }
        function.call_arg(args)
    }
}
