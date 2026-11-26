//! Rust API for modules linked into an Impasto application.

mod context;
mod diagnostics;
pub mod os;
mod os_channel;
mod runtime;
mod texture;
mod value;

use std::{collections::BTreeMap, fmt};

pub use context::{
    AppPaths, DiagnosticsSink, EventEmitter, Lifecycle, PathError, PluginContext, Subscription,
    WindowState,
};
pub use diagnostics::DiagnosticsSource;
pub use impasto_macros::{ImpastoEvents, ImpastoType, export, method, module, object};
pub use javascript::host::CancelToken;
pub use runtime::{BlockingTask, FutureTask, Runtime, RuntimeBackend, Task, TaskError};
#[cfg(target_os = "ios")]
pub use texture::MetalTexture;
#[doc(hidden)]
pub use texture::TextureFrame;
pub use texture::{ExternalFrame, ExternalTexture, ExternalTextureRegistry, TextureError};
pub use tracing;
pub use value::{
    Bytes, FromPluginValue, ImpastoType, IntoPluginValue, TsTypes, TypedSendError, TypedSink,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod abi {
    pub use javascript::host::*;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDescriptor {
    pub name: &'static str,
    pub specifier: &'static str,
    pub typescript: String,
    pub exports: Vec<&'static str>,
    /// Calling conventions indexed by JavaScript export.
    pub export_signatures: BTreeMap<&'static str, String>,
    #[doc(hidden)]
    pub selectable_exports: bool,
}

impl ModuleDescriptor {
    pub fn from_module(module: &abi::Module) -> Result<Self, PluginInitError> {
        Ok(Self {
            name: module.name,
            specifier: module.specifier,
            typescript: module
                .typescript_declaration()
                .map_err(PluginInitError::new)?,
            exports: module.export_names().collect(),
            export_signatures: module.export_signatures().map_err(PluginInitError::new)?,
            selectable_exports: false,
        })
    }
}

#[derive(Debug)]
pub struct PluginInitError(String);

impl PluginInitError {
    pub fn new(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }
}

impl fmt::Display for PluginInitError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(out)
    }
}

impl std::error::Error for PluginInitError {}

pub trait ImpastoError {
    fn scope(&self) -> &'static str;
    fn code(&self) -> &'static str;
    fn message(&self) -> String;
}

pub trait ImpastoPlugin: Send + Sync + 'static {
    fn describe(&self) -> Result<ModuleDescriptor, PluginInitError>;
    fn build_module(&self, ctx: &PluginContext) -> Result<abi::Module, PluginInitError>;

    #[doc(hidden)]
    fn selectable_exports(&self) -> bool {
        false
    }
}

pub trait ImpastoEvents: 'static {
    fn define_types(_types: &mut TsTypes) {}
    fn declarations() -> &'static [abi::EventType];
    fn into_event(self) -> Result<(&'static str, abi::PluginValue), String>;
}

pub type Register = fn(&mut PluginRegistrar);

#[derive(Default)]
pub struct PluginRegistrar {
    plugins: Vec<Box<dyn ImpastoPlugin>>,
}

impl PluginRegistrar {
    pub fn add(&mut self, plugin: impl ImpastoPlugin) {
        self.plugins.push(Box::new(plugin));
    }

    pub fn describe(&self) -> Result<Vec<ModuleDescriptor>, PluginInitError> {
        let mut names = std::collections::BTreeSet::new();
        let mut specifiers = std::collections::BTreeSet::new();
        self.plugins
            .iter()
            .map(|plugin| {
                let mut module = plugin.describe()?;
                module.selectable_exports = plugin.selectable_exports();
                if !names.insert(module.name) || !specifiers.insert(module.specifier) {
                    return Err(PluginInitError::new(format!(
                        "duplicate native module {} ({})",
                        module.specifier, module.name
                    )));
                }
                Ok(module)
            })
            .collect()
    }

    pub fn build(&self, ctx: &PluginContext) -> Result<Vec<abi::Module>, PluginInitError> {
        let descriptions = self.describe()?;
        self.plugins
            .iter()
            .zip(descriptions)
            .map(|(plugin, expected)| {
                let module = plugin
                    .build_module(&ctx.for_module(expected.name))
                    .map_err(|error| {
                        PluginInitError::new(format!("{}: {error}", expected.specifier))
                    })?;
                let mut actual = ModuleDescriptor::from_module(&module)?;
                actual.selectable_exports = plugin.selectable_exports();
                if actual != expected {
                    return Err(PluginInitError::new(format!(
                        "{}: runtime exports differ from the declaration",
                        expected.specifier
                    )));
                }
                Ok(module)
            })
            .collect()
    }
}

#[doc(hidden)]
pub mod macro_support {
    pub use crate::runtime::scoped;
    pub use crate::value::{decode_arg, decode_field, encode_error, finish_value};

    pub trait ExportSelection {
        const SPECIFIER: &'static str;
        const TABLE: Option<&'static str> = None;
    }

    pub struct AllExports;

    impl ExportSelection for AllExports {
        const SPECIFIER: &'static str = "";
    }

    /// The generated table lists only modules whose bindings should be trimmed.
    pub const fn native_export_enabled<S: ExportSelection>(name: &str) -> bool {
        let Some(table) = S::TABLE else {
            return true;
        };
        let bytes = table.as_bytes();
        let specifier = S::SPECIFIER.as_bytes();
        let name = name.as_bytes();
        let mut cursor = 0;
        let mut selected = false;
        while cursor < bytes.len() {
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\t' && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            let module = same_bytes(bytes, start, cursor, specifier);
            if cursor == bytes.len() || bytes[cursor] == b'\n' {
                cursor += 1;
                continue;
            }
            cursor += 1;
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            if module {
                selected = true;
                if same_bytes(bytes, start, cursor, name) {
                    return true;
                }
            }
            cursor += 1;
        }
        !selected
    }

    const fn same_bytes(bytes: &[u8], start: usize, end: usize, expected: &[u8]) -> bool {
        if end - start != expected.len() {
            return false;
        }
        let mut index = 0;
        while index < expected.len() {
            if bytes[start + index] != expected[index] {
                return false;
            }
            index += 1;
        }
        true
    }
}

#[doc(hidden)]
pub trait NativeObject: Sized + 'static {
    fn into_resource(self) -> Box<dyn abi::NativeResource>;
    fn typescript(types: &mut TsTypes) -> String;
}

#[doc(hidden)]
pub struct BoundResource {
    names: &'static [&'static str],
    bind: Option<Box<dyn FnOnce(CancelToken) -> Vec<abi::Method>>>,
    methods: Vec<abi::Method>,
}

impl BoundResource {
    pub fn new(
        names: &'static [&'static str],
        bind: impl FnOnce(CancelToken) -> Vec<abi::Method> + 'static,
    ) -> Self {
        Self {
            names,
            bind: Some(Box::new(bind)),
            methods: Vec::new(),
        }
    }
}

impl abi::NativeResource for BoundResource {
    fn methods(&self) -> &'static [&'static str] {
        self.names
    }

    fn method(&mut self, name: &str, cancel: &CancelToken) -> Option<abi::Method> {
        if let Some(bind) = self.bind.take() {
            self.methods = bind(cancel.clone());
        }
        self.methods
            .iter()
            .find(|method| method.name == name)
            .cloned()
    }

    fn call(
        &mut self,
        method: &str,
        _: &[abi::PluginValue],
        _: &CancelToken,
    ) -> Result<abi::PluginValue, String> {
        Err(format!("unknown resource method {method:?}"))
    }
}
