//! Native module ABI. Sync runs inline; Promise methods pick a [`MethodExec`].

use std::sync::Arc;

use super::{PluginResult, PluginValue};

pub type Work = Box<dyn FnOnce() -> PluginResult + Send>;

/// One-shot completion. Drop without [`Settle::send`] fails the promise.
pub struct Settle {
    send: Option<Box<dyn FnOnce(PluginResult) + Send>>,
}

impl Settle {
    pub fn new(send: impl FnOnce(PluginResult) + Send + 'static) -> Self {
        Self {
            send: Some(Box::new(send)),
        }
    }

    pub fn send(mut self, result: PluginResult) {
        if let Some(send) = self.send.take() {
            send(result);
        }
    }
}

impl Drop for Settle {
    fn drop(&mut self) {
        if let Some(send) = self.send.take() {
            send(PluginResult::Failed {
                message: "settle dropped".into(),
            });
        }
    }
}

pub trait MethodExec: Send + Sync {
    fn worker(&self, settle: Settle, work: Work);
    fn main_thread(&self, settle: Settle, work: Work);
    fn now(&self, settle: Settle, result: PluginResult);
}

/// Test executor. `worker` and `main_thread` run `work` on the caller.
pub struct ImmediateExec;

impl MethodExec for ImmediateExec {
    fn worker(&self, settle: Settle, work: Work) {
        settle.send(work());
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        settle.send(work());
    }

    fn now(&self, settle: Settle, result: PluginResult) {
        settle.send(result);
    }
}

pub type SyncFn = Arc<dyn Fn(&[PluginValue]) -> Result<PluginValue, String> + Send + Sync>;
pub type PromiseFn = Arc<dyn Fn(&[PluginValue], Settle, &dyn MethodExec) + Send + Sync>;

#[derive(Clone)]
pub enum Invoke {
    Sync(SyncFn),
    Promise(PromiseFn),
}

#[derive(Clone)]
pub struct Method {
    pub name: &'static str,
    pub invoke: Invoke,
    pub types: Option<MethodTypes>,
}

/// The JavaScript parameters and resolved return type of a native method.
#[derive(Clone, Copy)]
pub struct MethodTypes {
    pub parameters: &'static str,
    pub returns: &'static str,
}

impl Method {
    pub fn sync(
        name: &'static str,
        f: impl Fn(&[PluginValue]) -> Result<PluginValue, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Sync(Arc::new(f)),
            types: None,
        }
    }

    pub fn promise(
        name: &'static str,
        f: impl Fn(&[PluginValue], Settle, &dyn MethodExec) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Promise(Arc::new(f)),
            types: None,
        }
    }

    /// Describes the JavaScript signature. Promise methods specify the resolved value.
    #[must_use]
    pub fn with_types(mut self, parameters: &'static str, returns: &'static str) -> Self {
        self.types = Some(MethodTypes {
            parameters,
            returns,
        });
        self
    }
}

/// A native module event and the TypeScript type of its payload.
#[derive(Clone, Copy)]
pub struct EventType {
    pub name: &'static str,
    pub payload: &'static str,
}

#[derive(Clone, Copy, Default)]
pub struct ModuleExports {
    /// Also export an object named after the module, with the same functions.
    pub object: bool,
    pub typescript: &'static str,
    /// The events available to JS listeners. An empty list exports no listeners.
    pub events: &'static [EventType],
}

#[derive(Clone)]
pub struct Module {
    pub name: &'static str,
    pub specifier: &'static str,
    pub methods: Vec<Method>,
    pub exports: ModuleExports,
}

pub const LISTENER_EXPORTS: [&str; 2] = ["addEventListener", "removeEventListener"];

impl Module {
    /// Export names in declaration order, shared by the binder and compiler.
    pub fn export_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.methods
            .iter()
            .map(|method| method.name)
            .chain(self.listener_exports().iter().copied())
            .chain(self.exports.object.then_some(self.name))
    }

    pub fn listener_exports(&self) -> &'static [&'static str] {
        if self.exports.events.is_empty() {
            &[]
        } else {
            &LISTENER_EXPORTS
        }
    }
}

/// `module "<name>" is not linked`, or with the method when the name exists but the method does not.
pub fn module_not_linked(module: &str, method: Option<&str>) -> String {
    match method {
        Some(method) => format!("module \"{module}.{method}\" is not linked"),
        None => format!("module \"{module}\" is not linked"),
    }
}
