//! Persistent references owned by the render-thread reactive graph.
use super::{Inner, rt::Host};
use reactive::{CallResult, Eval, Graph as Arena, Id};
use rquickjs::{Ctx, Persistent, Value as JsValue};
use std::cell::RefCell;

#[derive(Clone)]
pub(super) struct Value {
    host: *mut Host,
    saved: Persistent<JsValue<'static>>,
}
impl Value {
    pub(super) fn save<'js>(ctx: &Ctx<'js>, host: *mut Host, value: JsValue<'js>) -> Self {
        Self {
            host,
            saved: Persistent::save(ctx, value),
        }
    }
    pub(super) fn restore<'js>(self, ctx: &Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
        self.saved.restore(ctx)
    }
}
#[derive(Clone, Copy)]
pub(super) struct JsEval;

pub(super) enum Cleanup {
    Script(Value),
    UnattachedNode(*mut Host, uicore::NodeId),
}

impl Eval for JsEval {
    type Value = Value;
    type Func = Value;
    type Cleanup = Cleanup;
    fn dup(self, value: &Value) -> Value {
        value.clone()
    }
    fn free(self, value: Value) {
        drop(value);
    }
    fn call(
        self,
        graph: &Arena<Self>,
        func: &Value,
        prev: Option<Value>,
    ) -> CallResult<Value, Cleanup> {
        // SAFETY: the session clears the graph while Host and the VM are alive.
        let host = unsafe { &*func.host };
        let marks = &host.inner().marks;
        let started = if marks.trace_enabled.get() {
            super::frame::bump(&marks.callbacks);
            (!marks.callback_active.replace(true)).then(std::time::Instant::now)
        } else {
            None
        };
        let result = host.with(|ctx| {
            let result = (|| {
                let function = rquickjs::Function::from_value(func.saved.clone().restore(&ctx)?)?;
                let mut args =
                    rquickjs::function::Args::new(ctx.clone(), usize::from(prev.is_some()));
                if let Some(prev) = prev {
                    args.push_arg(prev.restore(&ctx)?)?;
                }
                function.call_arg::<JsValue>(args)
            })();
            match result {
                Ok(value) => CallResult::Value {
                    value: Some(Value::save(&ctx, func.host, value)),
                    cleanup: None,
                },
                Err(error) => {
                    if !error.is_exception() {
                        rquickjs::Exception::throw_type(&ctx, &error.to_string());
                    }
                    host.reactive_error(&ctx, graph.owner().map_or(0, Id::to_raw));
                    CallResult::Failed
                }
            }
        });
        if let Some(started) = started {
            marks.callback_active.set(false);
            marks
                .callback_time
                .set(marks.callback_time.get() + started.elapsed());
        }
        result
    }
    fn effect(
        self,
        graph: &Arena<Self>,
        func: &Value,
        prev: Option<Value>,
    ) -> CallResult<Value, Cleanup> {
        match self.call(graph, func, prev) {
            CallResult::Value {
                value: Some(value), ..
            } => {
                // SAFETY: the graph is owned by this live Host.
                let is_function = unsafe { &*value.host }.with(|ctx| {
                    value
                        .saved
                        .clone()
                        .restore(&ctx)
                        .is_ok_and(|v| v.is_function())
                });
                CallResult::Value {
                    value: None,
                    cleanup: is_function.then_some(Cleanup::Script(value)),
                }
            }
            result => result,
        }
    }
    fn same(self, a: &Value, b: &Value) -> bool {
        // SAFETY: graph values belong to this Host, whose active VM entry holds its lock.
        unsafe { &*a.host }.same_saved(&a.saved, &b.saved)
    }

    fn run_cleanup(self, graph: &Arena<Self>, cleanup: Cleanup) {
        match cleanup {
            Cleanup::Script(cleanup) => {
                drop(graph.untrack(|graph| self.call(graph, &cleanup, None)));
            }
            Cleanup::UnattachedNode(host, id) => {
                // SAFETY: the session clears the graph before releasing Host.
                let host = unsafe { &*host };
                host.discard_if_unattached(id);
            }
        }
    }
}

pub(super) struct Graph {
    pub(super) arena: Arena<JsEval>,
    slots: RefCell<Vec<Option<Value>>>,
    free: RefCell<Vec<usize>>,
}
impl Graph {
    pub(super) fn new() -> Self {
        Self {
            arena: Arena::new(),
            slots: RefCell::new(vec![]),
            free: RefCell::new(vec![]),
        }
    }
    pub(super) fn hold(&self, value: Value) -> u64 {
        let index = self.free.borrow_mut().pop();
        let mut slots = self.slots.borrow_mut();
        let index = index.unwrap_or_else(|| {
            slots.push(None);
            slots.len() - 1
        });
        slots[index] = Some(value);
        index as u64 + 1
    }
    pub(super) fn take(&self, handle: u64) -> Option<Value> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        let value = self.slots.borrow_mut().get_mut(index)?.take()?;
        self.free.borrow_mut().push(index);
        Some(value)
    }
    pub(super) fn clear(&self) {
        // A cleanup may allocate another root. Retire it before releasing the VM.
        while self.arena.node_count() != 0 {
            self.arena.clear(JsEval);
        }
        self.slots.borrow_mut().clear();
        self.free.borrow_mut().clear();
    }
}
const RAW: u64 = 1 << 63;
fn raw(value: u64) -> u64 {
    RAW | value
}

pub(super) fn reactive_entry(inner: &Inner, op: u32, args: &[u64]) -> u64 {
    let arg = |i| args.get(i).copied().unwrap_or(0);
    let graph = &inner.graph.arena;
    let value = |i| inner.graph.take(arg(i));
    match op {
        1 => value(0).map_or(0, |v| raw(graph.create_signal(v).to_raw())),
        2 => graph
            .get(JsEval, Id::from_raw(arg(0)))
            .map_or(0, |v| inner.graph.hold(v)),
        3 => value(1).map_or(0, |v| {
            raw(u64::from(graph.set(JsEval, Id::from_raw(arg(0)), v)))
        }),
        4 => value(0).map_or(0, |f| raw(graph.create_memo(f).to_raw())),
        5 => {
            if let (Some(compute), Some(effect)) = (value(0), value(1)) {
                graph.render_effect(JsEval, compute, effect, arg(2) != 0);
            }
            0
        }
        6 => {
            if let Some(f) = value(0) {
                graph.create_tracked_effect(f);
            }
            0
        }
        7 => raw(graph
            .push_owner((arg(0) != 0).then(|| Id::from_raw(arg(0))))
            .map_or(0, Id::to_raw)),
        8 => {
            graph.dispose(JsEval, Id::from_raw(arg(0)));
            0
        }
        9 => raw(graph.owner().map_or(0, Id::to_raw)),
        10 => raw(graph.open_root().to_raw()),
        11 => {
            if let Some(f) = value(0) {
                graph.on_cleanup(Cleanup::Script(f));
            }
            0
        }
        12 => raw(graph.push_untrack().map_or(0, Id::to_raw)),
        13 => {
            graph.pop_reader((arg(0) != 0).then(|| Id::from_raw(arg(0))));
            0
        }
        14 => {
            graph.run_updates(JsEval);
            0
        }
        15 => {
            graph.push_batch();
            0
        }
        16 => {
            graph.pop_batch(JsEval);
            0
        }
        17 => raw(graph
            .owner_parent(Id::from_raw(arg(0)))
            .map_or(0, Id::to_raw)),
        18 => raw(u64::from(graph.is_live(Id::from_raw(arg(0))))),
        _ => 0,
    }
}
pub(super) fn release_all(inner: &Inner) {
    inner.graph.clear();
}
