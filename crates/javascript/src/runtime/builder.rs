//! Construction and partial installation of an owner-thread Runtime.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::host::{ImmediateExec, MethodExec};

use crate::runtime::ModuleCatalog;
use crate::runtime::modules::{MemoryLoader, MemoryResolver, NativeFactory};
use crate::runtime::quickjs::AcquireStop;
use crate::runtime::resources::Wake;
use crate::standard::StandardProfile;

use super::{
    Arc, Cell, CreditPool, Ctx, DrivePolicy, ErrorKind, HashMap, HttpClient, HttpMode, Inbox,
    IoBindings, JsError, Log, ModuleBindings, ModuleTable, Phase, Rc, RefCell, Resources, Runtime,
    RuntimeKey, RuntimeLimits, Scheduler, State, VecDeque, Vm,
};

pub struct RuntimeBuilder {
    sources: HashMap<String, String>,
    natives: HashMap<String, Box<dyn NativeFactory>>,
    catalog: ModuleCatalog,
    duplicate: Option<String>,
    io: IoBindings,
    log: Log,
    wake: Option<Wake>,
    profile: StandardProfile,
    limits: RuntimeLimits,
    policy: DrivePolicy,
    generation: u64,
    executor: Arc<dyn MethodExec>,
    stop: Option<BootStop>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BootStop {
    #[cfg(test)]
    Runtime,
    #[cfg(test)]
    Context,
    Bindings,
    Web,
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            natives: HashMap::new(),
            catalog: ModuleCatalog::default(),
            duplicate: None,
            io: IoBindings::jobs(),
            log: Box::new(|_, _| {}),
            wake: None,
            profile: StandardProfile::legacy(),
            limits: RuntimeLimits::default(),
            policy: DrivePolicy::LegacyStrict,
            generation: 1,
            executor: Arc::new(ImmediateExec),
            stop: None,
        }
    }

    pub fn log(mut self, log: impl Fn(u8, &str) + 'static) -> Self {
        self.log = Box::new(log);
        self
    }

    pub fn wake(mut self, wake: impl Fn() + Send + Sync + 'static) -> Self {
        self.wake = Some(Arc::new(wake));
        self
    }

    pub fn generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    pub fn standard(mut self, profile: StandardProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn limits(mut self, limits: RuntimeLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn drive_policy(mut self, policy: DrivePolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn modules(mut self, catalog: ModuleCatalog) -> Self {
        self.catalog = catalog;
        self
    }

    pub fn executor(mut self, exec: Arc<dyn MethodExec>) -> Self {
        self.executor = exec;
        self
    }

    pub fn host(mut self, io: IoBindings) -> Self {
        self.io = io;
        self
    }

    pub fn http_client(mut self, client: Arc<dyn HttpClient>) -> Self {
        self.io.http = HttpMode::Inline(client);
        self
    }

    pub fn module(mut self, name: impl Into<String>, source: impl Into<String>) -> Self {
        let name = name.into();
        if self.sources.contains_key(&name) || self.natives.contains_key(&name) {
            self.duplicate = Some(name);
        } else {
            self.sources.insert(name, source.into());
        }
        self
    }

    pub fn native(mut self, name: impl Into<String>, factory: Box<dyn NativeFactory>) -> Self {
        let name = name.into();
        if self.sources.contains_key(&name) || self.natives.contains_key(&name) {
            self.duplicate = Some(name);
        } else {
            self.natives.insert(name, factory);
        }
        self
    }

    pub fn build(self) -> Result<Runtime, JsError> {
        let stop = self.stop;
        let profile = self.profile.with_transports(
            matches!(self.io.http, HttpMode::External(_)),
            self.io.socket.is_some(),
        );
        let bindings =
            ModuleBindings::new(self.catalog.registry().clone(), Arc::clone(&self.executor));
        let runtime = self.build_unstarted()?;
        let table = Rc::clone(&runtime.state.modules);
        let state = &runtime.state;
        if let Some(vm) = state.vm.runtime() {
            vm.set_loader(
                MemoryResolver(Rc::clone(&table)),
                MemoryLoader(Rc::clone(&table)),
            );
        }
        let _budget = state.vm.budget("eval");
        state
            .vm
            .with(|ctx| {
                runtime.install_standard_bindings(&ctx, || {})?;
                runtime.install_host_inner(
                    &ctx,
                    |_: &Ctx<'_>| Ok(()),
                    move |name| table.contains(name),
                    stop == Some(BootStop::Bindings),
                )?;
                runtime.install_modules(&ctx, bindings, |_: &Ctx<'_>| Ok(()), None, None)?;
                crate::standard::install_web(&ctx, profile)?;
                if stop == Some(BootStop::Web) {
                    return Err(rquickjs::Exception::throw_internal(
                        &ctx,
                        "partial Web standard library installation",
                    ));
                }
                Ok(())
            })
            .map_err(|error| state.vm.with(|ctx| state.script_error(&ctx, error)))?;
        drop(_budget);
        runtime.finish_installation();
        Ok(runtime)
    }

    /// Allocate the VM and value codec. This extension entry has no loader,
    /// host globals, Web APIs or application script installed yet.
    pub fn build_unstarted(self) -> Result<Runtime, JsError> {
        if let Some(name) = self.duplicate {
            return Err(JsError::new(
                ErrorKind::Configuration,
                format!("module {name:?} is already registered"),
            ));
        }
        let limits = self.limits;
        if !limits.valid() || self.generation == 0 {
            return Err(JsError::new(
                ErrorKind::Configuration,
                "invalid runtime limits or generation",
            ));
        }
        let modules = ModuleTable::with_catalog(self.catalog, self.sources, self.natives)
            .map_err(|error| JsError::new(ErrorKind::Configuration, error.to_string()))?;
        let acquire: Option<AcquireStop> = match self.stop {
            #[cfg(test)]
            Some(BootStop::Runtime) => Some(AcquireStop::Runtime),
            #[cfg(test)]
            Some(BootStop::Context) => Some(AcquireStop::Context),
            _ => None,
        };
        let vm = Vm::configured(&limits, acquire)
            .map_err(|message| JsError::new(ErrorKind::Configuration, message))?;
        static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
        let instance = NEXT_INSTANCE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| JsError::new(ErrorKind::Capacity, "runtime IDs exhausted"))?;
        let key = RuntimeKey {
            instance,
            generation: self.generation,
        };
        let mut runtime = Runtime {
            state: Box::new(State {
                vm,
                key,
                limits,
                scheduler: Scheduler::default(),
                phase: Cell::new(Phase::Installing),
                life: RefCell::new(Some(Rc::new(()))),
                drive_policy: self.policy,
                modules: Rc::new(modules),
                tickets: RefCell::new(HashMap::new()),
                listeners: RefCell::new(Vec::new()),
                web_cleanup: RefCell::new(None),
                ticket_peak: Cell::new(0),
                listener_peak: Cell::new(0),
                log: self.log,
                host_policy: RefCell::new(Rc::new(|_| Ok(()))),
                io: RefCell::new(self.io),
                jobs: RefCell::new(Vec::new()),
                jobs_bytes: Cell::new(0),
                inbox: Arc::new(Inbox::new(key, self.wake)),
                local: RefCell::new(VecDeque::new()),
                resources: RefCell::new(Resources::default()),
                ingress: Arc::new(CreditPool::new(
                    limits.queue_bytes,
                    limits.queue_events,
                    limits.stream_bytes,
                    limits.stream_events,
                )),
                outbound: Arc::new(CreditPool::new(
                    limits.queue_bytes,
                    limits.queue_events,
                    limits.socket_send_bytes,
                    limits.stream_events,
                )),
            }),
        };
        // SAFETY: state is boxed and outlives all registered JavaScript functions.
        unsafe { Vm::arm(&mut runtime.state.vm) };
        let state = &runtime.state;
        let _budget = state.vm.budget("eval");
        state.vm.with(|ctx| {
            crate::runtime::quickjs::buffer::install(&ctx)
                .map_err(|error| state.script_error(&ctx, error))
        })?;
        drop(_budget);
        Ok(runtime)
    }

    pub fn bare() -> Result<Runtime, JsError> {
        Self::new()
            .host(IoBindings::disconnected())
            .build_unstarted()
    }
}

#[cfg(test)]
mod boot_tests {
    use super::*;

    #[test]
    fn each_boot_stop_releases_the_partial_runtime() {
        for (stop, expected) in [
            (BootStop::Runtime, [0, 0]),
            (BootStop::Context, [1, 0]),
            (BootStop::Bindings, [1, 1]),
            (BootStop::Web, [1, 1]),
        ] {
            let before = crate::runtime::quickjs::acquires();
            let mut builder = RuntimeBuilder::new();
            builder.stop = Some(stop);
            assert!(builder.build().is_err(), "{stop:?}");
            let after = crate::runtime::quickjs::acquires();
            for (index, count) in expected.into_iter().enumerate() {
                assert_eq!(after[index].0 - before[index].0, count, "{stop:?}");
                assert_eq!(after[index].1 - before[index].1, count, "{stop:?}");
            }
        }
    }
}
