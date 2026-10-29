//! Native module ABI. Sync runs inline; Promise methods pick a [`MethodExec`].

use std::sync::{Arc, RwLock};

use crate::io::{PluginResult, PluginValue, module_not_linked};

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

pub struct HostMethod {
    pub name: &'static str,
    pub invoke: Invoke,
}

impl HostMethod {
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

pub struct HostModule {
    pub name: &'static str,
    pub specifier: &'static str,
    pub methods: Vec<HostMethod>,
}

pub struct ModuleRegistry {
    modules: Arc<[HostModule]>,
}

impl ModuleRegistry {
    pub fn new(modules: Vec<HostModule>) -> Self {
        Self {
            modules: modules.into(),
        }
    }

    pub fn module(&self, name: &str) -> Option<&HostModule> {
        self.modules.iter().find(|module| module.name == name)
    }

    pub fn by_specifier(&self, specifier: &str) -> Option<&HostModule> {
        self.modules
            .iter()
            .find(|module| module.specifier == specifier)
    }

    pub fn method(&self, module: &str, method: &str) -> Option<&HostMethod> {
        self.module(module)?
            .methods
            .iter()
            .find(|entry| entry.name == method)
    }

    pub fn call(
        &self,
        module: &str,
        method: &str,
        args: &[PluginValue],
    ) -> Result<PluginValue, String> {
        match &self.method(module, method).map(|entry| &entry.invoke) {
            Some(Invoke::Sync(f)) => f(args),
            Some(Invoke::Promise(_)) => Err(format!("module \"{module}.{method}\" is not sync")),
            None => Err(module_not_linked(module, Some(method))),
        }
    }
}

/// Registry, executor, and shared slots installed on a language session.
#[derive(Clone)]
pub struct HostConfig {
    pub registry: Arc<ModuleRegistry>,
    pub exec: Arc<dyn MethodExec>,
    pub settle: Arc<dyn Fn(u64) -> Settle + Send + Sync>,
    pub app_state: Shared<String>,
    pub launch_url: Shared<Option<String>>,
}

impl HostConfig {
    /// Empty registry. Promise results are discarded.
    pub fn disconnected() -> Self {
        Self {
            registry: Arc::new(ModuleRegistry::new(Vec::new())),
            exec: Arc::new(ImmediateExec),
            settle: Arc::new(|_token| Settle::new(|_result| {})),
            app_state: shared("active".to_owned()),
            launch_url: shared(None),
        }
    }
}

pub type Shared<T> = Arc<RwLock<T>>;

pub fn shared<T>(value: T) -> Shared<T> {
    Arc::new(RwLock::new(value))
}

pub fn shared_read<T: Clone>(slot: &Shared<T>) -> T {
    slot.read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn shared_write<T>(slot: &Shared<T>, value: T) {
    *slot
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::PluginResult;

    #[test]
    fn settle_drop_sends_failed() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(Settle::new(move |result| {
            let _ = tx.send(result);
        }));
        match rx.recv() {
            Ok(PluginResult::Failed { message }) => assert_eq!(message, "settle dropped"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn settle_send_does_not_drop_twice() {
        let (tx, rx) = std::sync::mpsc::channel();
        Settle::new(move |result| {
            let _ = tx.send(result);
        })
        .send(PluginResult::Done(PluginValue::Null));
        match rx.recv() {
            Ok(PluginResult::Done(PluginValue::Null)) => {}
            other => panic!("{other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn immediate_exec_runs_on_the_caller() {
        let thread = std::thread::current().id();
        let exec = ImmediateExec;
        let (tx, rx) = std::sync::mpsc::channel();
        exec.worker(
            Settle::new({
                let tx = tx.clone();
                move |result| {
                    let _ = tx.send(("worker", result, std::thread::current().id()));
                }
            }),
            Box::new(|| PluginResult::Done(PluginValue::Str("w".into()))),
        );
        exec.main_thread(
            Settle::new({
                let tx = tx.clone();
                move |result| {
                    let _ = tx.send(("main", result, std::thread::current().id()));
                }
            }),
            Box::new(|| PluginResult::Done(PluginValue::Str("m".into()))),
        );
        exec.now(
            Settle::new(move |result| {
                let _ = tx.send(("now", result, std::thread::current().id()));
            }),
            PluginResult::Done(PluginValue::Str("n".into())),
        );
        for expected in ["worker", "main", "now"] {
            match rx.recv() {
                Ok((name, PluginResult::Done(PluginValue::Str(_)), id)) => {
                    assert_eq!(name, expected);
                    assert_eq!(id, thread);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn registry_first_wins_and_call_rejects_promise() {
        let first = HostModule {
            name: "Clipboard",
            specifier: "@uifw/clipboard/host",
            methods: vec![HostMethod::sync("getString", |_| {
                Ok(PluginValue::Str("one".into()))
            })],
        };
        let second = HostModule {
            name: "Clipboard",
            specifier: "@uifw/other/host",
            methods: vec![HostMethod::sync("getString", |_| {
                Ok(PluginValue::Str("two".into()))
            })],
        };
        let maps = HostModule {
            name: "Maps",
            specifier: "@uifw/maps/host",
            methods: vec![
                HostMethod::sync("ping", |_| Ok(PluginValue::Str("pong".into()))),
                HostMethod::promise("search", |_, settle, exec| {
                    exec.now(settle, PluginResult::Done(PluginValue::Null));
                }),
            ],
        };
        let registry = ModuleRegistry::new(vec![first, second, maps]);
        match registry.call("Clipboard", "getString", &[]) {
            Ok(PluginValue::Str(text)) => assert_eq!(text, "one"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            registry
                .by_specifier("@uifw/clipboard/host")
                .map(|module| module.name),
            Some("Clipboard")
        );
        match registry.call("Maps", "ping", &[]) {
            Ok(PluginValue::Str(text)) => assert_eq!(text, "pong"),
            other => panic!("{other:?}"),
        }
        match registry.call("Maps", "search", &[]) {
            Err(message) => assert!(message.contains("not sync"), "{message}"),
            other => panic!("{other:?}"),
        }
        match registry.call("Camera", "take", &[]) {
            Err(message) => assert!(message.contains("not linked"), "{message}"),
            other => panic!("{other:?}"),
        }
    }
}
