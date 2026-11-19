//! Module lookup. This registry does not know any package prefix.

use std::collections::HashSet;
use std::{fmt, sync::Arc};

use crate::declaration::{is_binding_identifier, is_identifier_name};
use crate::{Invoke, Method, Module, PluginValue, module_not_linked};

#[derive(Clone, Default)]
pub struct ModuleRegistry {
    modules: Arc<[Module]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    InvalidSpecifier(&'static str),
    DuplicateSpecifier(&'static str),
    InvalidName(&'static str),
    DuplicateName(&'static str),
    InvalidExport {
        module: &'static str,
        name: &'static str,
    },
    DuplicateExport {
        module: &'static str,
        name: &'static str,
    },
    InvalidEvent {
        module: &'static str,
        name: &'static str,
    },
    DuplicateEvent {
        module: &'static str,
        name: &'static str,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpecifier(name) => write!(out, "invalid module specifier {name:?}"),
            Self::DuplicateSpecifier(name) => write!(out, "duplicate module specifier {name:?}"),
            Self::InvalidName(name) => write!(out, "invalid module name {name:?}"),
            Self::DuplicateName(name) => write!(out, "duplicate module name {name:?}"),
            Self::InvalidExport { module, name } => {
                write!(out, "invalid export {name:?} in module {module:?}")
            }
            Self::DuplicateExport { module, name } => {
                write!(out, "duplicate export {name:?} in module {module:?}")
            }
            Self::InvalidEvent { module, name } => {
                write!(out, "invalid event {name:?} in module {module:?}")
            }
            Self::DuplicateEvent { module, name } => {
                write!(out, "duplicate event {name:?} in module {module:?}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Registered names are canonical virtual names. A relative import is resolved
/// against its parent first, before looking up the resulting registered name.
pub fn valid_specifier(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', '\0'])
        && !name.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

impl ModuleRegistry {
    /// Compatibility registry. Duplicate entries retain first-wins lookup.
    pub fn new(modules: Vec<Module>) -> Self {
        Self {
            modules: modules.into(),
        }
    }

    pub fn try_new(modules: Vec<Module>) -> Result<Self, RegistryError> {
        let registry = Self::new(modules);
        registry.validate()?;
        Ok(registry)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Module> {
        self.modules.iter()
    }

    pub fn validate(&self) -> Result<(), RegistryError> {
        let mut specifiers = HashSet::new();
        let mut names = HashSet::new();
        for module in self.iter() {
            validate_module(module)?;
            if !specifiers.insert(module.specifier) {
                return Err(RegistryError::DuplicateSpecifier(module.specifier));
            }
            if !names.insert(module.name) {
                return Err(RegistryError::DuplicateName(module.name));
            }
        }
        Ok(())
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

pub(crate) fn validate_module(module: &Module) -> Result<(), RegistryError> {
    if !valid_specifier(module.specifier) {
        return Err(RegistryError::InvalidSpecifier(module.specifier));
    }
    if module.name.is_empty()
        || module.name.chars().any(char::is_control)
        || module.exports.object && !is_binding_identifier(module.name)
    {
        return Err(RegistryError::InvalidName(module.name));
    }
    let mut exports = HashSet::new();
    for name in module.export_names() {
        if !is_identifier_name(name) {
            return Err(RegistryError::InvalidExport {
                module: module.name,
                name,
            });
        }
        if !exports.insert(name) {
            return Err(RegistryError::DuplicateExport {
                module: module.name,
                name,
            });
        }
    }
    let mut events = HashSet::new();
    for event in module.exports.events {
        let name = event.name;
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(RegistryError::InvalidEvent {
                module: module.name,
                name,
            });
        }
        if !events.insert(name) {
            return Err(RegistryError::DuplicateEvent {
                module: module.name,
                name,
            });
        }
    }
    Ok(())
}
