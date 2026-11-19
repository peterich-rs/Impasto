//! Explicit application module names and host module descriptions.

use std::{collections::BTreeMap, fmt, sync::Arc};

use js_host::{ModuleRegistry, RegistryError, valid_specifier};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogError {
    Registry(RegistryError),
    InvalidSpecifier(String),
    DuplicateSpecifier(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registry(error) => error.fmt(out),
            Self::InvalidSpecifier(name) => write!(out, "invalid module specifier {name:?}"),
            Self::DuplicateSpecifier(name) => write!(out, "module {name:?} is already registered"),
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Registry(error) => Some(error),
            Self::InvalidSpecifier(_) | Self::DuplicateSpecifier(_) => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct ModuleCatalog {
    registry: ModuleRegistry,
    pub(crate) sources: Arc<BTreeMap<String, String>>,
}

impl ModuleCatalog {
    /// Combines validated host modules with explicitly registered source modules.
    pub fn new(registry: ModuleRegistry) -> Result<Self, CatalogError> {
        registry.validate().map_err(CatalogError::Registry)?;
        Ok(Self {
            registry,
            sources: Arc::default(),
        })
    }

    pub fn add_source(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<(), CatalogError> {
        let name = name.into();
        self.check_available(&name)?;
        Arc::make_mut(&mut self.sources).insert(name, source.into());
        Ok(())
    }

    pub fn registry(&self) -> &ModuleRegistry {
        &self.registry
    }

    pub fn contains(&self, name: &str) -> bool {
        self.sources.contains_key(name) || self.registry.by_specifier(name).is_some()
    }

    pub(crate) fn check_available(&self, name: &str) -> Result<(), CatalogError> {
        if !valid_specifier(name) {
            Err(CatalogError::InvalidSpecifier(name.into()))
        } else if self.contains(name) {
            Err(CatalogError::DuplicateSpecifier(name.into()))
        } else {
            Ok(())
        }
    }
}
