//! Native module ABI. Sync runs inline; Promise methods pick a [`MethodExec`].

use std::sync::{Arc, RwLock};

use crate::io::PluginValue;

pub use js_host::{
    EventType, ImmediateExec, Invoke, Method, MethodExec, MethodTypes, Module, ModuleExports,
    PromiseFn, Settle, SyncFn, Work,
};

/// Compatibility registry. The modules live in one [`js_host::ModuleRegistry`].
///
/// `restricted` is the `@impasto/` package filter. The generic registry has no
/// such policy, and this wrapper does not implement `Deref`.
pub struct ModuleRegistry {
    inner: js_host::ModuleRegistry,
}

impl ModuleRegistry {
    pub fn new(modules: Vec<Module>) -> Self {
        Self {
            inner: js_host::ModuleRegistry::new(modules),
        }
    }

    pub fn restricted(&self, packages: &[String]) -> Self {
        Self {
            inner: self.inner.filtered(|module| {
                packages.iter().any(|package| {
                    module.specifier.strip_prefix("@impasto/") == Some(package.as_str())
                })
            }),
        }
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.inner.module(name)
    }

    pub fn by_specifier(&self, specifier: &str) -> Option<&Module> {
        self.inner.by_specifier(specifier)
    }

    pub fn method(&self, module: &str, method: &str) -> Option<&Method> {
        self.inner.method(module, method)
    }

    pub fn call(
        &self,
        module: &str,
        method: &str,
        args: &[PluginValue],
    ) -> Result<PluginValue, String> {
        self.inner.call(module, method, args)
    }

    pub fn as_host_registry(&self) -> &js_host::ModuleRegistry {
        &self.inner
    }
}

/// Registry, executor, and shared slots installed on a language session.
#[derive(Clone)]
pub struct ModuleConfig {
    pub registry: Arc<ModuleRegistry>,
    pub exec: Arc<dyn MethodExec>,
    pub settle: Arc<dyn Fn(u64) -> Settle + Send + Sync>,
    pub app_state: Shared<String>,
    pub launch_url: Shared<Option<String>>,
}

impl ModuleConfig {
    pub fn restrict(mut self, packages: &[String]) -> Self {
        self.registry = Arc::new(self.registry.restricted(packages));
        self
    }

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
        let first = Module {
            name: "Clipboard",
            exports: Default::default(),
            specifier: "@impasto/clipboard",
            methods: vec![Method::sync("getString", |_| {
                Ok(PluginValue::Str("one".into()))
            })],
        };
        let second = Module {
            name: "Clipboard",
            exports: Default::default(),
            specifier: "@impasto/other",
            methods: vec![Method::sync("getString", |_| {
                Ok(PluginValue::Str("two".into()))
            })],
        };
        let maps = Module {
            name: "Maps",
            exports: Default::default(),
            specifier: "@impasto/maps",
            methods: vec![
                Method::sync("ping", |_| Ok(PluginValue::Str("pong".into()))),
                Method::promise("search", |_, settle, exec| {
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
                .by_specifier("@impasto/clipboard")
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
