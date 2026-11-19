//! Native module ABI. Sync runs inline; Promise methods pick a [`MethodExec`].

use std::sync::Arc;

use crate::{PluginResult, PluginValue};

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
}

impl Method {
    pub fn sync(
        name: &'static str,
        f: impl Fn(&[PluginValue]) -> Result<PluginValue, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Sync(Arc::new(f)),
        }
    }

    pub fn promise(
        name: &'static str,
        f: impl Fn(&[PluginValue], Settle, &dyn MethodExec) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Promise(Arc::new(f)),
        }
    }
}

#[derive(Clone)]
pub struct Module {
    pub name: &'static str,
    pub specifier: &'static str,
    pub methods: Vec<Method>,
}

/// `module "<name>" is not linked`, or with the method when the name exists but the method does not.
pub fn module_not_linked(module: &str, method: Option<&str>) -> String {
    match method {
        Some(method) => format!("module \"{module}.{method}\" is not linked"),
        None => format!("module \"{module}\" is not linked"),
    }
}
