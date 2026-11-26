//! Reads and writes documents in the application sandbox.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use impasto::abi::PluginValue;
use impasto::os::NativeChannel;
use impasto::{
    Bytes, FromPluginValue, ImpastoError, ImpastoType, PluginContext, PluginInitError,
    PluginRegistrar, TsTypes,
};

mod files;
#[cfg(target_os = "macos")]
mod macos;

#[derive(Debug)]
struct FileError(String);

impl ImpastoError for FileError {
    fn scope(&self) -> &'static str {
        "file-system"
    }
    fn code(&self) -> &'static str {
        "io"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

struct RelativePath(String);

impl FromPluginValue for RelativePath {
    fn from_value(value: PluginValue) -> Result<Self, String> {
        let path = String::from_value(value)?;
        files::document_relative(&path).map(Self)
    }
}

impl impasto::IntoPluginValue for RelativePath {
    fn into_value(self) -> Result<PluginValue, String> {
        self.0.into_value()
    }
}

impl ImpastoType for RelativePath {
    fn typescript(_: &mut TsTypes) -> String {
        "string".into()
    }
}

#[derive(ImpastoType)]
struct PickedDocument {
    name: String,
}

#[impasto::module(name = "FileSystem", specifier = "@impasto/file-system/native", init = init)]
struct FileSystemPlugin {
    root: Option<PathBuf>,
    channel: Option<Arc<NativeChannel>>,
    #[cfg(target_os = "macos")]
    runtime: Option<impasto::Runtime>,
}

impl FileSystemPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            root: ctx.paths().document_dir().ok().map(Path::to_owned),
            channel: ctx.extension(),
            #[cfg(target_os = "macos")]
            runtime: ctx.is_hosted().then(|| ctx.runtime().clone()),
        })
    }

    fn root(&self) -> Result<&Path, FileError> {
        self.root
            .as_deref()
            .ok_or_else(|| FileError("the document directory is unavailable".into()))
    }
}

#[impasto::export]
impl FileSystemPlugin {
    pub fn document_directory(&self) -> Result<String, FileError> {
        Ok(self.root()?.to_string_lossy().into_owned())
    }

    #[impasto::method(blocking)]
    pub fn read(&self, path: RelativePath) -> Result<Bytes, FileError> {
        files::read_document(self.root()?, &path.0)
            .map(Bytes)
            .map_err(FileError)
    }

    #[impasto::method(blocking)]
    pub fn write(&self, path: RelativePath, bytes: Bytes) -> Result<(), FileError> {
        files::write_document(self.root()?, &path.0, &bytes.0).map_err(FileError)
    }

    #[impasto::method(blocking)]
    pub fn delete(&self, path: RelativePath) -> Result<(), FileError> {
        files::delete_document(self.root()?, &path.0).map_err(FileError)
    }

    #[impasto::method(awaits_user)]
    pub async fn pick(&self) -> Result<Option<PickedDocument>, FileError> {
        self.root()?;
        #[cfg(target_os = "macos")]
        if let Some(runtime) = &self.runtime {
            return macos::pick(runtime, self.root()?.to_owned())
                .await
                .map(|name| name.map(|name| PickedDocument { name }))
                .map_err(FileError);
        }
        let Some(channel) = &self.channel else {
            return Ok(None);
        };
        let Some(name) = channel
            .request("@impasto/file-system/pick", "")
            .await
            .map_err(FileError)?
        else {
            return Ok(None);
        };
        let name = files::document_relative(&name).map_err(FileError)?;
        Ok(Some(PickedDocument { name }))
    }
}

/// Registers the document-directory bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(FileSystemPlugin::plugin());
}
