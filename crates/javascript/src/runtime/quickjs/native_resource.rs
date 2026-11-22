//! Native module resources stay on the runtime owner thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rquickjs::class::Trace;
use rquickjs::function::{Rest, This};
use rquickjs::{Class, Ctx, Exception, JsLifetime, Result, Value};

use crate::host::{CancelToken, NativeResource};
use crate::runtime::resources::next_id;

use super::codec::{decode_args, encode};

#[derive(Default)]
pub(crate) struct ResourceTable {
    entries: RefCell<HashMap<u64, Entry>>,
}

struct Entry {
    value: Box<dyn NativeResource>,
    cancel: CancelToken,
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl ResourceTable {
    pub fn close(&self) {
        let entries = std::mem::take(&mut *self.entries.borrow_mut());
        drop(entries);
    }

    pub fn open<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        cap: usize,
        create: impl FnOnce() -> std::result::Result<Box<dyn NativeResource>, String>,
    ) -> Result<Value<'js>> {
        if self.entries.borrow().len() >= cap {
            return Err(Exception::throw_range(
                ctx,
                "native resource limit exceeded",
            ));
        }
        let id = next_id()
            .ok_or_else(|| Exception::throw_range(ctx, "native resource IDs exhausted"))?;
        let value = native_call(ctx, create)?;
        self.entries.borrow_mut().insert(
            id,
            Entry {
                value,
                cancel: CancelToken::default(),
            },
        );
        let lease = Lease {
            id,
            table: Rc::clone(self),
        };
        Class::instance(ctx.clone(), NativeHandle { lease }).map(Class::into_value)
    }

    fn remove(&self, id: u64) {
        let entry = self.entries.borrow_mut().remove(&id);
        drop(entry);
    }
}

struct Lease {
    id: u64,
    table: Rc<ResourceTable>,
}

// SAFETY: the lease owns only native Rust state, independent of the JS lifetime.
unsafe impl<'js> JsLifetime<'js> for Lease {
    type Changed<'to> = Self;
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.table.remove(self.id);
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "NativeResource")]
struct NativeHandle {
    #[qjs(skip_trace)]
    lease: Lease,
}

#[rquickjs::methods]
impl NativeHandle {
    fn close(&self) {
        self.lease.table.remove(self.lease.id);
    }

    fn call<'js>(
        This(handle): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        method: String,
        Rest(args): Rest<Value<'js>>,
    ) -> Result<Value<'js>> {
        let args = decode_args(&ctx, args)?;
        let handle = handle.try_borrow()?;
        let value = {
            let mut entries = handle
                .lease
                .table
                .entries
                .try_borrow_mut()
                .map_err(|_| Exception::throw_type(&ctx, "native resource is in use"))?;
            let entry = entries
                .get_mut(&handle.lease.id)
                .ok_or_else(|| Exception::throw_type(&ctx, "native resource is closed"))?;
            native_call(&ctx, || entry.value.call(&method, &args, &entry.cancel))?
        };
        encode(&ctx, &value)
    }
}

pub(super) fn native_call<T>(
    ctx: &Ctx<'_>,
    work: impl FnOnce() -> std::result::Result<T, String>,
) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .map_err(|_| Exception::throw_type(ctx, "native callback panicked"))?
        .map_err(|message| Exception::throw_type(ctx, &message))
}
