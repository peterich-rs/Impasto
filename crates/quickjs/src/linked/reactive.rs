//! QuickJS references owned by the render-thread arena. Re-entry borrows no
//! mutable session/arena state. Each JSValue has exactly one owning `Value`.
use super::Inner;
use reactive::{CallResult, Eval, Graph as Arena, Id};
use std::cell::RefCell;
use std::ffi::c_void;

type Bytes = [u8; 16];
pub(super) struct Value {
    ctx: *mut c_void,
    bytes: Bytes,
}
impl Clone for Value {
    fn clone(&self) -> Self {
        let mut bytes = [0; 16];
        // SAFETY: Value owns a live reference; its interpreter outlives the graph.
        unsafe { qjs_reactive_dup(self.ctx, self.bytes.as_ptr(), bytes.as_mut_ptr()) };
        Self {
            ctx: self.ctx,
            bytes,
        }
    }
}
impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: session clears the graph and transfer slots before qjs_close.
        unsafe { qjs_reactive_free(self.ctx, self.bytes.as_ptr()) };
    }
}
#[derive(Clone, Copy)]
pub(super) struct JsEval;
impl Eval for JsEval {
    type Value = Value;
    type Func = Value;
    type Cleanup = Value;
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
    ) -> CallResult<Value, Value> {
        let mut bytes = [0; 16];
        // SAFETY: C borrows both references for JS_Call; the returned reference
        // transfers to Value. JS callbacks may freely re-enter the graph.
        let failed = unsafe {
            qjs_reactive_call(
                func.ctx,
                func.bytes.as_ptr(),
                prev.as_ref().map_or(std::ptr::null(), |v| v.bytes.as_ptr()),
                bytes.as_mut_ptr(),
            )
        };
        if failed != 0 {
            // SAFETY: the failed JS_Call left an exception in the same context.
            unsafe { qjs_reactive_handle_error(func.ctx, graph.owner().map_or(0, Id::to_raw)) };
            CallResult::Failed
        } else {
            CallResult::Value {
                value: Some(Value {
                    ctx: func.ctx,
                    bytes,
                }),
                cleanup: None,
            }
        }
    }
    fn effect(
        self,
        graph: &Arena<Self>,
        func: &Value,
        prev: Option<Value>,
    ) -> CallResult<Value, Value> {
        match self.call(graph, func, prev) {
            CallResult::Value {
                value: Some(value), ..
            } => {
                // SAFETY: value is a live JSValue in this context.
                if unsafe { qjs_reactive_is_fn(value.ctx, value.bytes.as_ptr()) } != 0 {
                    CallResult::Value {
                        value: None,
                        cleanup: Some(value),
                    }
                } else {
                    drop(value);
                    CallResult::Value {
                        value: None,
                        cleanup: None,
                    }
                }
            }
            result => result,
        }
    }
    fn same(self, a: &Value, b: &Value) -> bool {
        // SAFETY: both values belong to this session and stay alive for the call.
        unsafe { qjs_reactive_same(a.ctx, a.bytes.as_ptr(), b.bytes.as_ptr()) != 0 }
    }
    fn run_cleanup(self, graph: &Arena<Self>, cleanup: Value) {
        let result = graph.untrack(|graph| self.call(graph, &cleanup, None));
        drop(result);
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
    fn hold(&self, value: Value) -> u64 {
        let index = self.free.borrow_mut().pop();
        let mut slots = self.slots.borrow_mut();
        let index = index.unwrap_or_else(|| {
            slots.push(None);
            slots.len() - 1
        });
        slots[index] = Some(value);
        index as u64 + 1
    }
    fn take(&self, handle: u64) -> Option<Value> {
        let index = usize::try_from(handle.checked_sub(1)?).ok()?;
        let value = self.slots.borrow_mut().get_mut(index)?.take()?;
        self.free.borrow_mut().push(index);
        Some(value)
    }
    pub(super) fn clear(&self) {
        self.arena.clear(JsEval);
        self.slots.borrow_mut().clear();
        self.free.borrow_mut().clear();
    }
}
const RAW: u64 = 1 << 63;
fn raw(value: u64) -> u64 {
    RAW | value
}

// These functions are private to glue.c; no Rust type crosses the ABI.
#[unsafe(no_mangle)]
pub(super) unsafe extern "C" fn reactive_hold(
    session: *mut c_void,
    ctx: *mut c_void,
    bytes: *const u8,
) -> u64 {
    super::qjs_entry!(0, {
        if session.is_null() || ctx.is_null() || bytes.is_null() {
            return 0;
        }
        let mut copied = [0; 16];
        // SAFETY: glue supplies one JSValue in a 16-byte buffer and transfers ownership.
        unsafe { std::ptr::copy_nonoverlapping(bytes, copied.as_mut_ptr(), 16) };
        let inner = unsafe { &*session.cast::<Inner>() };
        inner.graph.hold(Value { ctx, bytes: copied })
    })
}
#[unsafe(no_mangle)]
pub(super) unsafe extern "C" fn reactive_take(
    session: *mut c_void,
    handle: u64,
    out: *mut u8,
) -> i32 {
    super::qjs_entry!(-1, {
        if session.is_null() || out.is_null() {
            return -1;
        }
        // SAFETY: session is the live Inner supplied in QjsFns.
        let inner = unsafe { &*session.cast::<Inner>() };
        let Some(value) = inner.graph.take(handle) else {
            return -1;
        };
        // SAFETY: out has 16 bytes. Ownership transfers to C and then QuickJS.
        unsafe { std::ptr::copy_nonoverlapping(value.bytes.as_ptr(), out, 16) };
        std::mem::forget(value);
        0
    })
}
#[unsafe(no_mangle)]
pub(super) unsafe extern "C" fn reactive_entry(
    session: *mut c_void,
    _ctx: *mut c_void,
    op: u32,
    args: *const u64,
    argc: u32,
) -> u64 {
    super::qjs_entry!(0, {
        if session.is_null() {
            return 0;
        }
        // SAFETY: Inner lives until qjs_close; args is a stack array in glue.c.
        let inner = unsafe { &*session.cast::<Inner>() };
        let args = if args.is_null() {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(args, argc as usize) }
        };
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
                    graph.on_cleanup(f);
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
    })
}
pub(super) fn release_all(inner: &Inner) {
    inner.graph.clear();
}
unsafe extern "C" {
    fn qjs_reactive_handle_error(ctx: *mut c_void, owner: u64) -> i32;
    fn qjs_reactive_dup(ctx: *mut c_void, bytes: *const u8, out: *mut u8);
    fn qjs_reactive_free(ctx: *mut c_void, bytes: *const u8);
    fn qjs_reactive_call(ctx: *mut c_void, func: *const u8, arg: *const u8, out: *mut u8) -> i32;
    fn qjs_reactive_same(ctx: *mut c_void, a: *const u8, b: *const u8) -> i32;
    fn qjs_reactive_is_fn(ctx: *mut c_void, bytes: *const u8) -> i32;
}
