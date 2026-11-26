use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use crate::{ExternalTextureRegistry, ImpastoEvents, Runtime, abi::HostEvent};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Lifecycle {
    #[default]
    Resumed,
    Inactive,
    Hidden,
    Paused,
    Detached,
}

#[derive(Clone, Default)]
pub struct AppPaths {
    document: Option<PathBuf>,
    data: Option<PathBuf>,
    cache: Option<PathBuf>,
    temp: Option<PathBuf>,
}

impl AppPaths {
    pub fn new(document: Option<PathBuf>, cache: Option<PathBuf>, temp: Option<PathBuf>) -> Self {
        Self {
            document,
            data: None,
            cache,
            temp,
        }
    }

    pub fn document_dir(&self) -> Result<&Path, PathError> {
        self.document.as_deref().ok_or(PathError("document"))
    }

    #[doc(hidden)]
    pub fn with_data_dir(mut self, data: Option<PathBuf>) -> Self {
        self.data = data;
        self
    }

    /// Returns the private application data directory, using documents where they coincide.
    pub fn data_dir(&self) -> Result<&Path, PathError> {
        self.data
            .as_deref()
            .or(self.document.as_deref())
            .ok_or(PathError("data"))
    }

    pub fn cache_dir(&self) -> Result<&Path, PathError> {
        self.cache.as_deref().ok_or(PathError("cache"))
    }

    pub fn temp_dir(&self) -> Result<&Path, PathError> {
        self.temp.as_deref().ok_or(PathError("temp"))
    }
}

#[derive(Debug)]
pub struct PathError(&'static str);

impl std::fmt::Display for PathError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "the {} directory is unavailable", self.0)
    }
}

impl std::error::Error for PathError {}

type EventPoster = Arc<dyn Fn(HostEvent) -> bool + Send + Sync>;
type Extension = Arc<dyn Any + Send + Sync>;
type Handler<T> = Arc<dyn Fn(T) + Send + Sync>;

struct Listeners<T> {
    next: u64,
    handlers: BTreeMap<u64, Handler<T>>,
    closed: bool,
}

impl<T> Default for Listeners<T> {
    fn default() -> Self {
        Self {
            next: 0,
            handlers: BTreeMap::new(),
            closed: false,
        }
    }
}

type Signal<T> = Mutex<Listeners<T>>;

/// Keeps a window event handler registered until dropped.
#[must_use]
pub struct Subscription(Option<Box<dyn FnOnce() + Send + Sync>>);

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(remove) = self.0.take() {
            remove();
        }
    }
}

fn subscribe<T: 'static>(signal: &Arc<Signal<T>>, handler: Handler<T>) -> Subscription {
    let mut listeners = signal.lock().unwrap_or_else(|error| error.into_inner());
    let key = listeners.next;
    if listeners.closed {
        return Subscription(None);
    }
    let Some(next) = key.checked_add(1) else {
        return Subscription(None);
    };
    listeners.next = next;
    listeners.handlers.insert(key, handler);
    let signal = Arc::downgrade(signal);
    Subscription(Some(Box::new(move || {
        if let Some(signal) = Weak::upgrade(&signal) {
            let removed = signal
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .handlers
                .remove(&key);
            drop(removed);
        }
    })))
}

fn handlers<T>(signal: &Signal<T>) -> Vec<Handler<T>> {
    signal
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .handlers
        .values()
        .cloned()
        .collect()
}

fn close<T>(signal: &Signal<T>) -> Option<BTreeMap<u64, Handler<T>>> {
    let mut listeners = signal.lock().unwrap_or_else(|error| error.into_inner());
    if std::mem::replace(&mut listeners.closed, true) {
        return None;
    }
    Some(std::mem::take(&mut listeners.handlers))
}

/// Receives package telemetry on behalf of a window.
pub trait DiagnosticsSink: Send + Sync + 'static {
    fn record(&self, source: &str, event: &str, payload: crate::abi::PluginValue);
}

/// A key-value store owned by a platform window or window group.
pub trait WindowState: Send + Sync + 'static {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: Option<&str>) -> Result<(), String>;
}

#[derive(Clone, Default)]
pub struct PluginContext {
    runtime: Runtime,
    paths: AppPaths,
    lifecycle: Arc<Signal<Lifecycle>>,
    lifecycle_state: Arc<Mutex<Lifecycle>>,
    focus: Arc<Signal<bool>>,
    focused: Arc<AtomicBool>,
    urls: Arc<Signal<Arc<str>>>,
    window_changes: Arc<Signal<Arc<str>>>,
    initial_url: Option<Arc<str>>,
    window_session: Option<Arc<dyn WindowState>>,
    restoration: Option<Arc<dyn WindowState>>,
    module: &'static str,
    events: Option<TypeId>,
    post: Option<EventPoster>,
    extensions: Arc<Mutex<Extensions>>,
    diagnostics: Option<Arc<dyn DiagnosticsSink>>,
    textures: ExternalTextureRegistry,
    #[cfg(target_os = "android")]
    pub(crate) android: Option<crate::os::android::AndroidContext>,
}

#[derive(Default)]
struct Extensions {
    values: HashMap<TypeId, Extension>,
    closed: bool,
}

impl PluginContext {
    pub fn new(
        runtime: Runtime,
        paths: AppPaths,
        post: impl Fn(HostEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            runtime,
            paths,
            post: Some(Arc::new(post)),
            ..Self::default()
        }
    }

    /// Publishes a service to all modules in this window session.
    /// Returns the previous instance of the same type, if any.
    pub fn insert_extension<T: Any + Send + Sync>(&self, value: T) -> Option<Arc<T>> {
        let previous = {
            let mut extensions = self
                .extensions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if extensions.closed {
                return None;
            }
            extensions.values.insert(TypeId::of::<T>(), Arc::new(value))
        };
        previous?.downcast().ok()
    }

    /// Shares a service registered by the host or an earlier module.
    pub fn extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let value = self
            .extensions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values
            .get(&TypeId::of::<T>())
            .cloned()?;
        value.downcast().ok()
    }

    #[doc(hidden)]
    pub fn with_initial_url(mut self, url: Option<String>) -> Self {
        self.initial_url = url.filter(|url| !url.is_empty()).map(Arc::from);
        self
    }

    /// Returns the URL that started this session, if one was supplied.
    pub fn initial_url(&self) -> Option<&str> {
        self.initial_url.as_deref()
    }

    /// Registers for URLs delivered after session startup.
    pub fn on_open_url(&self, handler: impl Fn(&str) + Send + Sync + 'static) -> Subscription {
        subscribe(&self.urls, Arc::new(move |url| handler(&url)))
    }

    /// Indicates whether this window currently has input focus.
    pub fn is_window_focused(&self) -> bool {
        self.focused.load(Ordering::Acquire)
    }

    /// Registers for changes to this window's input focus.
    pub fn on_window_focus(&self, handler: impl Fn(bool) + Send + Sync + 'static) -> Subscription {
        subscribe(&self.focus, Arc::new(handler))
    }

    #[doc(hidden)]
    pub fn with_diagnostics(mut self, sink: Arc<dyn DiagnosticsSink>) -> Self {
        self.diagnostics = Some(sink);
        self
    }

    /// Returns the optional telemetry sink for this window.
    pub fn diagnostics(&self) -> Option<&dyn DiagnosticsSink> {
        self.diagnostics.as_deref()
    }

    /// Shares the optional telemetry sink with an independently running task.
    pub fn diagnostics_sink(&self) -> Option<Arc<dyn DiagnosticsSink>> {
        self.diagnostics.clone()
    }

    /// Indicates whether this context belongs to an attached application host.
    pub fn is_hosted(&self) -> bool {
        self.runtime.has_backend()
    }

    #[doc(hidden)]
    pub fn with_window_state(
        mut self,
        session: impl WindowState,
        restoration: Option<impl WindowState>,
    ) -> Self {
        self.window_session = Some(Arc::new(session));
        self.restoration = restoration.map(|state| Arc::new(state) as _);
        self
    }

    /// State shared by windows belonging to the same application session.
    pub fn window_session(&self) -> Option<&dyn WindowState> {
        self.window_session.as_deref()
    }

    /// State restored for this individual window by its platform.
    pub fn restoration(&self) -> Option<&dyn WindowState> {
        self.restoration.as_deref()
    }

    /// Registers for changes to keys in the shared window session.
    pub fn on_window_state(&self, handler: impl Fn(&str) + Send + Sync + 'static) -> Subscription {
        subscribe(&self.window_changes, Arc::new(move |key| handler(&key)))
    }

    pub fn with_textures(mut self, textures: ExternalTextureRegistry) -> Self {
        self.textures = textures;
        self
    }

    pub fn external_textures(&self) -> &ExternalTextureRegistry {
        &self.textures
    }

    #[cfg(target_os = "android")]
    pub fn with_android(mut self, context: crate::os::android::AndroidContext) -> Self {
        self.android = Some(context);
        self
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    #[doc(hidden)]
    pub fn with_event_schema<E: ImpastoEvents>(&self) -> Self {
        Self {
            events: Some(TypeId::of::<E>()),
            ..self.clone()
        }
    }

    pub fn emitter<E: ImpastoEvents>(&self) -> EventEmitter<E> {
        EventEmitter {
            module: self.module,
            post: self
                .post
                .clone()
                .filter(|_| self.events == Some(TypeId::of::<E>())),
            events: PhantomData,
        }
    }

    pub fn on_lifecycle(
        &self,
        handler: impl Fn(Lifecycle) + Send + Sync + 'static,
    ) -> Subscription {
        subscribe(&self.lifecycle, Arc::new(handler))
    }

    /// Returns the last lifecycle state observed by the application window.
    pub fn lifecycle(&self) -> Lifecycle {
        *self
            .lifecycle_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    #[doc(hidden)]
    pub fn dispatch_lifecycle(&self, event: Lifecycle) {
        {
            let mut state = self
                .lifecycle_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *state == Lifecycle::Detached {
                return;
            }
            *state = event;
        }
        if event == Lifecycle::Detached {
            let Some(lifecycle) = close(&self.lifecycle) else {
                return;
            };
            drop(close(&self.focus));
            self.focused.store(false, Ordering::Release);
            drop(close(&self.urls));
            drop(close(&self.window_changes));
            for handler in lifecycle.into_values() {
                handler(event);
            }
            self.textures.close();
            self.runtime.close();
            let values = {
                let mut extensions = self
                    .extensions
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                extensions.closed = true;
                std::mem::take(&mut extensions.values)
            };
            drop(values);
        } else {
            for handler in handlers(&self.lifecycle) {
                handler(event);
            }
        }
    }

    #[doc(hidden)]
    pub fn dispatch_window_focus(&self, focused: bool) {
        let handlers = {
            let listeners = self.focus.lock().unwrap_or_else(|error| error.into_inner());
            if listeners.closed || self.focused.swap(focused, Ordering::AcqRel) == focused {
                return;
            }
            listeners.handlers.values().cloned().collect::<Vec<_>>()
        };
        for handler in handlers {
            handler(focused);
        }
    }

    #[doc(hidden)]
    pub fn dispatch_open_url(&self, url: &str) {
        if !url.is_empty() {
            let url = Arc::<str>::from(url);
            for handler in handlers(&self.urls) {
                handler(Arc::clone(&url));
            }
        }
    }

    #[doc(hidden)]
    pub fn dispatch_window_state(&self, key: &str) {
        let key = Arc::<str>::from(key);
        for handler in handlers(&self.window_changes) {
            handler(Arc::clone(&key));
        }
    }

    pub(crate) fn for_module(&self, module: &'static str) -> Self {
        Self {
            module,
            ..self.clone()
        }
    }
}

pub struct EventEmitter<E> {
    module: &'static str,
    post: Option<EventPoster>,
    events: PhantomData<fn(E)>,
}

impl<E: ImpastoEvents> EventEmitter<E> {
    pub fn emit(&self, event: E) -> Result<(), String> {
        let (event, payload) = event.into_event()?;
        let post = self
            .post
            .as_ref()
            .filter(|_| !self.module.is_empty())
            .ok_or("module has no event host")?;
        if post(HostEvent {
            module: self.module.into(),
            event: event.into(),
            payload,
        }) {
            Ok(())
        } else {
            Err("event host is closed or its queue is full".into())
        }
    }
}
