//! Persistent string storage for the current application.

use std::path::{Path, PathBuf};

use impasto::{PluginContext, PluginInitError, PluginRegistrar};

mod store;

#[impasto::module(
    name = "AsyncStorage",
    specifier = "@impasto/async-storage/native",
    init = init,
    object = true
)]
struct AsyncStoragePlugin {
    root: Option<PathBuf>,
}

impl AsyncStoragePlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            root: ctx.paths().data_dir().ok().map(Path::to_owned),
        })
    }

    fn root(&self) -> Result<&Path, StorageError> {
        self.root
            .as_deref()
            .ok_or_else(|| StorageError("the application data directory is unavailable".into()))
    }
}

#[derive(Debug)]
struct StorageError(String);

impl impasto::ImpastoError for StorageError {
    fn scope(&self) -> &'static str {
        "async-storage"
    }
    fn code(&self) -> &'static str {
        "storage"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

#[impasto::export]
impl AsyncStoragePlugin {
    #[impasto::method(blocking)]
    pub fn multi_get(
        &self,
        keys: Vec<String>,
    ) -> Result<Vec<(String, Option<String>)>, StorageError> {
        store::store_get(self.root()?, &keys).map_err(StorageError)
    }

    #[impasto::method(blocking)]
    pub fn multi_set(&self, entries: Vec<(String, String)>) -> Result<(), StorageError> {
        store::store_set(self.root()?, &entries).map_err(StorageError)
    }

    #[impasto::method(blocking)]
    pub fn multi_remove(&self, keys: Vec<String>) -> Result<(), StorageError> {
        store::store_remove(self.root()?, &keys).map_err(StorageError)
    }

    #[impasto::method(blocking)]
    pub fn clear(&self) -> Result<(), StorageError> {
        store::store_clear(self.root()?).map_err(StorageError)
    }
}

/// Adds the application's storage API to its native registry.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(AsyncStoragePlugin::plugin());
}
