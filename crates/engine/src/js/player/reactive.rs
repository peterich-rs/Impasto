//! Persistent references owned by the render-thread reactive graph.
use super::{Inner, vm::Host};
use crate::reactive::{CallResult, Eval, Graph as Arena, Id};
use rquickjs::{Ctx, Persistent, Value as JsValue};
use std::rc::Weak;

#[derive(Clone)]
pub(super) struct Value {
    host: Weak<Host>,
    saved: Persistent<JsValue<'static>>,
}
impl Value {
    pub(super) fn save<'js>(ctx: &Ctx<'js>, host: &Host, value: JsValue<'js>) -> Self {
        Self {
            host: host.downgrade(),
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
    UnattachedNode(Weak<Host>, crate::tree::NodeId),
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
        let Some(host) = func.host.upgrade() else {
            return CallResult::Failed;
        };
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
                    value: Some(Value::save(&ctx, &host, value)),
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
                let is_function = value.host.upgrade().is_some_and(|host| {
                    host.with(|ctx| {
                        value
                            .saved
                            .clone()
                            .restore(&ctx)
                            .is_ok_and(|v| v.is_function())
                    })
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
        a.host
            .upgrade()
            .is_some_and(|host| host.same_saved(&a.saved, &b.saved))
    }

    fn run_cleanup(self, graph: &Arena<Self>, cleanup: Cleanup) {
        match cleanup {
            Cleanup::Script(cleanup) => {
                if let Some(host) = cleanup.host.upgrade()
                    && (!host.is_closing() || !host.past_deadline())
                {
                    drop(graph.untrack(|graph| self.call(graph, &cleanup, None)));
                }
            }
            Cleanup::UnattachedNode(host, id) => {
                if let Some(host) = host.upgrade() {
                    host.discard_if_unattached(id);
                }
            }
        }
    }
}

pub(super) type Graph = Arena<JsEval>;

pub(super) fn release_all(inner: &Inner) {
    // A cleanup may allocate another root. Retire it before releasing the VM.
    while inner.graph.node_count() != 0 {
        inner.graph.clear(JsEval);
    }
}
