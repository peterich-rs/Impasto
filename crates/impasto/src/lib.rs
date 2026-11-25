//! Rust API for modules linked into an Impasto application.

mod context;
mod http;
pub mod os;
mod runtime;
mod texture;
mod value;

use std::fmt;

pub use context::{AppPaths, EventEmitter, Lifecycle, PathError, PluginContext, Subscription};
#[doc(hidden)]
pub use http::HttpStreamSender;
pub use http::{HttpChunk, HttpClient, HttpError, HttpRequest, HttpResponse, HttpStream};
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
                let module = plugin.describe()?;
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
                if ModuleDescriptor::from_module(&module)? != expected {
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
