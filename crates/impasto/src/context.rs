use std::any::TypeId;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use crate::{ExternalTextureRegistry, HttpClient, ImpastoEvents, Runtime, abi::HostEvent};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Resumed,
    Inactive,
    Hidden,
    Paused,
    Detached,
}

#[derive(Clone, Default)]
pub struct AppPaths {
    document: Option<PathBuf>,
    cache: Option<PathBuf>,
    temp: Option<PathBuf>,
}

impl AppPaths {
    pub fn new(document: Option<PathBuf>, cache: Option<PathBuf>, temp: Option<PathBuf>) -> Self {
        Self {
            document,
            cache,
            temp,
        }
    }

    pub fn document_dir(&self) -> Result<&Path, PathError> {
        self.document.as_deref().ok_or(PathError("document"))
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

type Handler = Arc<dyn Fn(Lifecycle) + Send + Sync>;
type EventPoster = Arc<dyn Fn(HostEvent) -> bool + Send + Sync>;

#[derive(Default)]
struct Listeners {
    next: u64,
    handlers: BTreeMap<u64, Handler>,
    closed: bool,
}

#[must_use]
pub struct Subscription {
    listeners: Weak<Mutex<Listeners>>,
    key: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(listeners) = self.listeners.upgrade() {
            let removed = listeners
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .handlers
                .remove(&self.key);
            drop(removed);
        }
    }
}

#[derive(Clone, Default)]
pub struct PluginContext {
    runtime: Runtime,
    paths: AppPaths,
    listeners: Arc<Mutex<Listeners>>,
    module: &'static str,
    events: Option<TypeId>,
    post: Option<EventPoster>,
    http: Option<HttpClient>,
    textures: ExternalTextureRegistry,
    #[cfg(target_os = "android")]
    pub(crate) android: Option<crate::os::android::AndroidContext>,
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

    pub fn with_http(mut self, client: HttpClient) -> Self {
        self.http = Some(client);
        self
    }

    pub fn http(&self) -> Option<&HttpClient> {
        self.http.as_ref()
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
        let mut listeners = self
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let key = listeners.next;
        if !listeners.closed
            && let Some(next) = key.checked_add(1)
        {
            listeners.next = next;
            listeners.handlers.insert(key, Arc::new(handler));
        }
        Subscription {
            listeners: Arc::downgrade(&self.listeners),
            key,
        }
    }

    pub fn dispatch_lifecycle(&self, event: Lifecycle) {
        let handlers = {
            let mut listeners = self
                .listeners
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if listeners.closed {
                return;
            }
            if event == Lifecycle::Detached {
                listeners.closed = true;
                std::mem::take(&mut listeners.handlers)
                    .into_values()
                    .collect::<Vec<_>>()
            } else {
                listeners.handlers.values().cloned().collect()
            }
        };
        for handler in handlers {
            handler(event);
        }
        if event == Lifecycle::Detached {
            self.textures.close();
            self.runtime.close();
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
