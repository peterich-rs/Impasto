//! Native module ABI. Sync runs inline; Promise methods pick a [`MethodExec`].

use std::sync::Arc;

use super::{NativeResource, PluginResult, PluginValue, StreamSink};

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
pub type ResourceFn =
    Arc<dyn Fn(&[PluginValue]) -> Result<Box<dyn NativeResource>, String> + Send + Sync>;
pub type StreamFn = Arc<dyn Fn(Vec<PluginValue>, StreamSink, &dyn MethodExec) + Send + Sync>;
pub type PromiseFn = Arc<dyn Fn(&[PluginValue], Settle, &dyn MethodExec) + Send + Sync>;

#[derive(Clone)]
pub enum Invoke {
    Sync(SyncFn),
    Promise(PromiseFn),
    Resource(ResourceFn),
    Stream(StreamFn),
}

#[derive(Clone)]
pub struct Method {
    pub name: &'static str,
    pub invoke: Invoke,
    pub types: Option<MethodTypes>,
    pub(crate) awaits_user: bool,
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
            awaits_user: false,
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
            awaits_user: false,
        }
    }

    /// Open a managed, thread-affine resource. JS can call methods or close it.
    pub fn resource(
        name: &'static str,
        open: impl Fn(&[PluginValue]) -> Result<Box<dyn NativeResource>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Resource(Arc::new(open)),
            types: None,
            awaits_user: false,
        }
    }

    /// Start a producer on the native executor. JS may pass an AbortSignal as
    /// its last argument. The producer receives the decoded remaining arguments.
    pub fn stream(
        name: &'static str,
        work: impl Fn(Vec<PluginValue>, &mut StreamSink) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        let work = Arc::new(work);
        Self::stream_with_exec(name, move |args, mut sink, exec| {
            let work = Arc::clone(&work);
            exec.worker(
                Settle::new(|_| {}),
                Box::new(move || {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        work(args, &mut sink)
                    })) {
                        Ok(Ok(())) => sink.finish(),
                        Ok(Err(message)) => sink.fail(message),
                        Err(_) => sink.fail("native stream producer panicked".into()),
                    }
                    PluginResult::Done(PluginValue::Null)
                }),
            );
        })
    }

    /// Start a stream producer that schedules its own work on `exec` or an async runtime.
    pub fn stream_with_exec(
        name: &'static str,
        start: impl Fn(Vec<PluginValue>, StreamSink, &dyn MethodExec) + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            invoke: Invoke::Stream(Arc::new(start)),
            types: None,
            awaits_user: false,
        }
    }

    /// A platform prompt completes when the user responds or the owner detaches.
    /// Keep its request slot while it is open and leave cancellation to the host.
    #[must_use]
    pub fn awaits_user(mut self) -> Self {
        self.awaits_user = true;
        self
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
