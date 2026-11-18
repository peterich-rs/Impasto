//! Module lookup. This registry does not know any package prefix.

use std::sync::Arc;

use crate::{Invoke, Method, Module, PluginValue, module_not_linked};

pub struct ModuleRegistry {
    modules: Arc<[Module]>,
}

impl ModuleRegistry {
    pub fn new(modules: Vec<Module>) -> Self {
        Self {
            modules: modules.into(),
        }
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|module| module.name == name)
    }

    pub fn by_specifier(&self, specifier: &str) -> Option<&Module> {
        self.modules
            .iter()
            .find(|module| module.specifier == specifier)
    }

    pub fn method(&self, module: &str, method: &str) -> Option<&Method> {
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
            Some(Invoke::Sync(function)) => function(args),
            Some(Invoke::Promise(_)) => Err(format!("module \"{module}.{method}\" is not sync")),
            None => Err(module_not_linked(module, Some(method))),
        }
    }

    /// Modules that `keep` accepts, in their original order.
    ///
    /// Callbacks stay in their existing [`Arc`]s. Entries are not deduplicated
    /// or reordered, so the first match still wins a later lookup.
    pub fn filtered(&self, mut keep: impl FnMut(&Module) -> bool) -> Self {
        Self::new(
            self.modules
                .iter()
                .filter(|module| keep(module))
                .cloned()
                .collect(),
        )
    }
}
