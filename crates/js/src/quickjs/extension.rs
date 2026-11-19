//! Scoped access to QuickJS for trusted native extensions.

use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Coerced, Ctx, FromJs, Value};

use super::vm::{Budget, JsDiagnostic, Saved, Vm};

/// Borrowed access to the backend of a live Runtime. The owner installs
/// native modules before entering the application and releases its roots
/// before the Runtime closes.
#[derive(Clone, Copy)]
pub struct ExtensionContext<'a> {
    vm: &'a Vm,
}

impl<'a> ExtensionContext<'a> {
    pub(crate) fn new(vm: &'a Vm) -> Self {
        Self { vm }
    }

    pub fn is_open(self) -> bool {
        self.vm.is_open()
    }

    pub fn with<R>(self, body: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        self.vm.with(body)
    }

    /// Use the context already on the native callback stack for nested entry.
    pub fn with_current<R>(self, ctx: &Ctx<'_>, body: impl FnOnce() -> R) -> R {
        assert!(
            self.vm.owns(ctx),
            "extension context belongs to another runtime"
        );
        let _active = self.vm.enter(ctx);
        body()
    }

    pub fn budget(self, phase: &'static str, id: u64) -> Budget<'a> {
        let budget = self.vm.budget(phase);
        self.retarget(phase, id);
        budget
    }

    /// Update the diagnostic label without extending the deadline.
    pub fn retarget(self, phase: &'static str, id: u64) {
        self.vm.entry.set((phase, id));
    }

    pub fn past_deadline(self) -> bool {
        crate::monotonic_ms() > self.vm.deadline.get()
    }

    pub fn install_loader(
        self,
        resolver: impl Resolver + 'static,
        loader: impl Loader + 'static,
    ) -> rquickjs::Result<()> {
        if self.vm.is_active() {
            return Err(rquickjs::Error::new_loading_message(
                "extension loader",
                "VM is executing",
            ));
        }
        let runtime = self.vm.runtime().ok_or_else(|| {
            rquickjs::Error::new_loading_message("extension loader", "runtime is closed")
        })?;
        runtime.set_loader(resolver, loader);
        Ok(())
    }

    pub fn diagnostic(self) -> JsDiagnostic {
        self.vm.diagnostic()
    }

    pub fn record_error(self, message: String, stack: String) {
        *self.vm.error.borrow_mut() = (self.vm.annotate(message), stack);
    }

    pub fn stash(self, ctx: &Ctx<'_>, error: rquickjs::Error) {
        if error.is_exception() {
            self.stash_value(ctx, ctx.catch());
        } else {
            self.record_error(error.to_string(), String::new());
        }
    }

    pub fn stash_value<'js>(self, ctx: &Ctx<'js>, value: Value<'js>) {
        let (message, stack) = exception_text(ctx, value);
        self.record_error(message, stack);
    }

    pub fn take_rejections(self) -> Vec<Saved> {
        std::mem::take(&mut *self.vm.rejections.borrow_mut())
            .into_iter()
            .map(|(_, reason)| reason)
            .collect()
    }
}

/// Diagnostics may read throwing getters or call a custom toString. Keep the
/// original value and consume secondary exceptions within the current budget.
pub fn exception_text<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> (String, String) {
    let stack = value
        .as_object()
        .and_then(|object| object.get::<_, Option<Coerced<String>>>("stack").ok())
        .flatten()
        .map(|s| s.0)
        .unwrap_or_default();
    if ctx.has_exception() {
        drop(ctx.catch());
    }
    let message = Coerced::<String>::from_js(ctx, value)
        .map(|s| s.0)
        .unwrap_or_else(|_| "quickjs error".to_owned());
    if ctx.has_exception() {
        drop(ctx.catch());
    }
    (message, stack)
}
