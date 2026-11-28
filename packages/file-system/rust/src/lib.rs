//! Scoped file I/O and native document pickers.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    AppPaths, Bytes, CancelToken, ImpastoError, ImpastoType, PluginContext, PluginInitError,
    PluginRegistrar, Runtime, TypedSink,
};
use serde::{Deserialize, Serialize};

mod files;
pub use files::FileWriter;

/// Directories that can be used by a relative file path.
#[derive(Clone, Copy, Debug, Default, ImpastoType, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    #[default]
    Documents,
    ApplicationSupport,
    Cache,
    Temp,
}

/// Standard directories provided by the current application host.
#[derive(Clone, Debug, ImpastoType)]
pub struct AppDirectories {
    pub documents: String,
    pub application_support: String,
    pub cache: String,
    pub temp: String,
    #[impasto(nullable)]
    pub downloads: Option<String>,
}

#[derive(Debug, ImpastoType)]
pub enum FileKind {
    File,
    Directory,
}

#[derive(Debug, ImpastoType)]
pub struct FileStat {
    kind: FileKind,
    size: f64,
    #[impasto(nullable)]
    modified_at_ms: Option<f64>,
    #[impasto(nullable)]
    created_at_ms: Option<f64>,
    readonly: bool,
}

#[derive(Debug, ImpastoType)]
pub struct DirEntry {
    name: String,
    path: String,
    kind: FileKind,
    size: f64,
}

/// A path, filesystem, or native document picker failure.
#[derive(Debug)]
pub struct FileError {
    code: &'static str,
    message: String,
}

impl FileError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn outside(path: &str) -> Self {
        Self::new(
            "outside-scope",
            format!("path is outside the selected directory: {path:?}"),
        )
    }
    fn io(error: impl fmt::Display) -> Self {
        Self::new("io", error.to_string())
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message)
    }
}

impl std::error::Error for FileError {}

impl ImpastoError for FileError {
    fn scope(&self) -> &'static str {
        "file-system"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

impl From<io::Error> for FileError {
    fn from(error: io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::NotFound => "not-found",
            io::ErrorKind::AlreadyExists => "already-exists",
            io::ErrorKind::NotADirectory => "not-a-directory",
            io::ErrorKind::IsADirectory => "not-a-file",
            io::ErrorKind::PermissionDenied => "permission-denied",
            _ => "io",
        };
        Self::new(code, error.to_string())
    }
}

impl From<NativePluginError> for FileError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "cancelled" => "cancelled",
            "host-unavailable" => "host-unavailable",
            "permission-denied" => "permission-denied",
            _ => "io",
        };
        Self::new(code, error.message)
    }
}

fn root(paths: &AppPaths, scope: Scope) -> Result<&Path, FileError> {
    match scope {
        Scope::Documents => paths.document_dir(),
        Scope::ApplicationSupport => paths.data_dir(),
        Scope::Cache => paths.cache_dir(),
        Scope::Temp => paths.temp_dir(),
    }
    .map_err(|error| FileError::new("host-unavailable", error.to_string()))
}

fn paths_to_directories(paths: &AppPaths) -> Result<AppDirectories, FileError> {
    let path = |scope| Ok::<_, FileError>(root(paths, scope)?.to_string_lossy().into_owned());
    Ok(AppDirectories {
        documents: path(Scope::Documents)?,
        application_support: path(Scope::ApplicationSupport)?,
        cache: path(Scope::Cache)?,
        temp: path(Scope::Temp)?,
        downloads: paths
            .downloads_dir()
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
    })
}

/// Returns the application directories supplied by the host.
pub fn directories(ctx: &PluginContext) -> Result<AppDirectories, FileError> {
    paths_to_directories(ctx.paths())
}

/// Validates a relative filename against its scope, including any existing symlinks.
pub fn resolve_path(
    ctx: &PluginContext,
    scope: Scope,
    relative: &str,
) -> Result<PathBuf, FileError> {
    files::resolve(root(ctx.paths(), scope)?, relative)
}

#[derive(Default, ImpastoType)]
pub struct WriteOptions {
    scope: Option<Scope>,
    atomic: Option<bool>,
}

#[derive(Default, ImpastoType)]
pub struct DirectoryOptions {
    scope: Option<Scope>,
    recursive: Option<bool>,
}

#[derive(Default, ImpastoType)]
pub struct TransferOptions {
    from_scope: Option<Scope>,
    to_scope: Option<Scope>,
    overwrite: Option<bool>,
}

#[derive(Default, ImpastoType)]
pub struct ReadStreamOptions {
    scope: Option<Scope>,
    chunk_size: Option<u32>,
}

#[derive(Default, ImpastoType)]
pub struct WriterOptions {
    scope: Option<Scope>,
    append: Option<bool>,
    atomic: Option<bool>,
}

#[derive(Default, ImpastoType, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickFilesOptions {
    multiple: Option<bool>,
    accept: Option<Vec<String>>,
}

#[derive(ImpastoType, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFile {
    name: String,
    relative_path: String,
    size: f64,
    #[impasto(nullable)]
    mime_type: Option<String>,
}

#[derive(ImpastoType, Deserialize)]
pub struct PickedDirectory {
    name: String,
    path: String,
}

#[derive(ImpastoType)]
pub struct SaveFileOptions {
    suggested_name: String,
    mime_type: Option<String>,
    allowed_extensions: Option<Vec<String>>,
    bytes: Option<Bytes>,
    source_path: Option<String>,
    scope: Option<Scope>,
}

#[impasto::module(name = "FileSystem", specifier = "@impasto/file-system/native", init = init)]
struct FileSystemPlugin {
    paths: AppPaths,
    platform: NativePlugin,
    runtime: Runtime,
}

impl FileSystemPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            paths: ctx.paths().clone(),
            platform: NativePlugin::new(ctx, "@impasto/file-system"),
            runtime: ctx.runtime().clone(),
        })
    }

    fn resolve(&self, scope: Option<Scope>, path: &str) -> Result<PathBuf, FileError> {
        files::resolve(root(&self.paths, scope.unwrap_or_default())?, path)
    }

    async fn io<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, FileError> + Send + 'static,
    ) -> Result<T, FileError> {
        if self.platform.is_hosted() {
            self.runtime
                .spawn_blocking(work)
                .await
                .map_err(|error| FileError::new("cancelled", error.to_string()))?
        } else {
            work()
        }
    }

    fn put(
        &self,
        path: &str,
        data: &[u8],
        scope: Option<Scope>,
        append: bool,
        atomic: bool,
    ) -> Result<(), FileError> {
        let writer = FileWriter::open(self.resolve(scope, path)?, append, atomic)?;
        writer.write_bytes(data)?;
        writer.commit()
    }
}

#[impasto::export]
impl FileSystemPlugin {
    pub fn is_file_system_host_available(&self) -> bool {
        self.platform.is_hosted()
    }

    pub fn document_directory(&self) -> Result<String, FileError> {
        Ok(root(&self.paths, Scope::Documents)?
            .to_string_lossy()
            .into_owned())
    }

    pub fn directories(&self) -> Result<AppDirectories, FileError> {
        paths_to_directories(&self.paths)
    }

    #[impasto::method(blocking)]
    pub fn exists(&self, path: String, scope: Option<Scope>) -> Result<bool, FileError> {
        Ok(self.resolve(scope, &path)?.try_exists()?)
    }

    #[impasto::method(blocking)]
    pub fn stat(&self, path: String, scope: Option<Scope>) -> Result<FileStat, FileError> {
        files::stat(&self.resolve(scope, &path)?)
    }

    #[impasto::method(blocking)]
    pub fn list_dir(
        &self,
        path: Option<String>,
        scope: Option<Scope>,
    ) -> Result<Vec<DirEntry>, FileError> {
        files::list(
            root(&self.paths, scope.unwrap_or_default())?,
            path.as_deref().unwrap_or_default(),
        )
    }

    #[impasto::method(blocking)]
    pub fn mkdir(&self, path: String, options: Option<DirectoryOptions>) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        let path = self.resolve(options.scope, &path)?;
        if options.recursive == Some(true) {
            fs::create_dir_all(path)?;
        } else {
            fs::create_dir(path)?;
        }
        Ok(())
    }

    #[impasto::method(blocking)]
    pub fn read(&self, path: String, scope: Option<Scope>) -> Result<Bytes, FileError> {
        let mut bytes = Vec::new();
        files::open_file(&self.resolve(scope, &path)?)?.read_to_end(&mut bytes)?;
        Ok(Bytes(bytes))
    }

    #[impasto::method(blocking)]
    pub fn read_text(&self, path: String, scope: Option<Scope>) -> Result<String, FileError> {
        let mut text = String::new();
        files::open_file(&self.resolve(scope, &path)?)?.read_to_string(&mut text)?;
        Ok(text)
    }

    #[impasto::method(blocking)]
    pub fn write(
        &self,
        path: String,
        bytes: Bytes,
        options: Option<WriteOptions>,
    ) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        self.put(
            &path,
            &bytes.0,
            options.scope,
            false,
            options.atomic.unwrap_or(false),
        )
    }

    #[impasto::method(blocking)]
    pub fn write_text(
        &self,
        path: String,
        text: String,
        options: Option<WriteOptions>,
    ) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        self.put(
            &path,
            text.as_bytes(),
            options.scope,
            false,
            options.atomic.unwrap_or(false),
        )
    }

    #[impasto::method(blocking)]
    pub fn append_bytes(
        &self,
        path: String,
        bytes: Bytes,
        scope: Option<Scope>,
    ) -> Result<(), FileError> {
        self.put(&path, &bytes.0, scope, true, false)
    }

    #[impasto::method(blocking)]
    pub fn append_text(
        &self,
        path: String,
        text: String,
        scope: Option<Scope>,
    ) -> Result<(), FileError> {
        self.put(&path, text.as_bytes(), scope, true, false)
    }

    #[impasto::method(blocking)]
    pub fn copy(
        &self,
        from: String,
        to: String,
        options: Option<TransferOptions>,
    ) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        files::copy(
            &self.resolve(options.from_scope, &from)?,
            &self.resolve(options.to_scope, &to)?,
            options.overwrite.unwrap_or(false),
        )
    }

    #[impasto::method(blocking)]
    pub fn move_path(
        &self,
        from: String,
        to: String,
        options: Option<TransferOptions>,
    ) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        files::move_entry(
            &self.resolve(options.from_scope, &from)?,
            &self.resolve(options.to_scope, &to)?,
            options.overwrite.unwrap_or(false),
        )
    }

    #[impasto::method(blocking)]
    pub fn delete(&self, path: String, options: Option<DirectoryOptions>) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        let path = self.resolve(options.scope, &path)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {
                if options.recursive == Some(true) {
                    fs::remove_dir_all(path)?;
                } else {
                    fs::remove_dir(path)?;
                }
            }
            Ok(_) => fs::remove_file(path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    pub async fn read_stream(
        &self,
        path: String,
        options: Option<ReadStreamOptions>,
        mut sink: TypedSink<Bytes>,
        cancel: CancelToken,
    ) -> Result<(), FileError> {
        let options = options.unwrap_or_default();
        let chunk_size = (options.chunk_size.unwrap_or(65_536) as usize)
            .min(1_048_576)
            .min(sink.max_piece_bytes());
        if chunk_size == 0 {
            return Err(FileError::new("io", "chunk size must be positive"));
        }
        let scope = root(&self.paths, options.scope.unwrap_or_default())?.to_owned();
        let mut file = self
            .io(move || files::open_file(&files::resolve(&scope, &path)?))
            .await?;
        while !cancel.is_cancelled() {
            let (next, bytes) = self
                .io(move || {
                    let mut buffer = vec![0; chunk_size];
                    let length = file.read(&mut buffer)?;
                    buffer.truncate(length);
                    Ok((file, buffer))
                })
                .await?;
            file = next;
            if bytes.is_empty() || sink.send(Bytes(bytes)).await.is_err() {
                break;
            }
        }
        Ok(())
    }

    #[impasto::method(resource)]
    pub async fn open_writer(
        &self,
        path: String,
        options: Option<WriterOptions>,
    ) -> Result<FileWriter, FileError> {
        let options = options.unwrap_or_default();
        let base = root(&self.paths, options.scope.unwrap_or_default())?.to_owned();
        self.io(move || {
            FileWriter::open(
                files::resolve(&base, &path)?,
                options.append.unwrap_or(false),
                options.atomic.unwrap_or(false),
            )
        })
        .await
    }

    #[impasto::method(awaits_user)]
    pub async fn pick_files(
        &self,
        options: Option<PickFilesOptions>,
    ) -> Result<Vec<PickedFile>, FileError> {
        if !self.platform.is_hosted() {
            return Ok(Vec::new());
        }
        let files: Vec<PickedFile> = self
            .platform
            .request("pickFiles", &options.unwrap_or_default())
            .await?;
        for file in &files {
            self.resolve(None, &file.relative_path)?;
        }
        Ok(files)
    }

    #[impasto::method(awaits_user)]
    pub async fn pick_directory(&self) -> Result<Option<PickedDirectory>, FileError> {
        if !self.platform.is_hosted() {
            return Ok(None);
        }
        Ok(self.platform.request("pickDirectory", &()).await?)
    }

    #[impasto::method(awaits_user)]
    pub async fn save_file(&self, options: SaveFileOptions) -> Result<bool, FileError> {
        files::relative_path(&options.suggested_name)?;
        if options.suggested_name.contains('/')
            || options.bytes.is_some() == options.source_path.is_some()
        {
            return Err(FileError::new(
                "io",
                "saveFile requires a filename and exactly one data source",
            ));
        }
        if !self.platform.is_hosted() {
            return Ok(false);
        }
        let extensions = options
            .allowed_extensions
            .unwrap_or_default()
            .into_iter()
            .map(|extension| {
                let extension = extension.strip_prefix('.').unwrap_or(&extension);
                if extension.is_empty()
                    || !extension
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                {
                    return Err(FileError::new("io", "invalid save file extension"));
                }
                Ok(extension.to_ascii_lowercase())
            })
            .collect::<Result<Vec<_>, FileError>>()?;
        let paths = self.paths.clone();
        let mut suggested_name = options.suggested_name;
        if Path::new(&suggested_name).extension().is_none()
            && let Some(extension) = extensions.first()
        {
            suggested_name.push('.');
            suggested_name.push_str(extension);
        }
        let name = suggested_name.clone();
        let directory = self
            .io(move || {
                let staging = tempfile::tempdir_in(root(&paths, Scope::Cache)?)?;
                let mut file = fs::File::create(staging.path().join(name))?;
                if let Some(bytes) = options.bytes {
                    file.write_all(&bytes.0)?;
                } else if let Some(path) = options.source_path {
                    let source =
                        files::resolve(root(&paths, options.scope.unwrap_or_default())?, &path)?;
                    io::copy(&mut files::open_file(&source)?, &mut file)?;
                }
                Ok(staging)
            })
            .await?;
        let payload = serde_json::json!({ "path": directory.path().join(&suggested_name), "suggestedName": suggested_name, "mimeType": options.mime_type, "allowedExtensions": extensions });
        Ok(self.platform.request("saveFile", &payload).await?)
    }
}

/// Registers the sandbox filesystem bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(FileSystemPlugin::plugin());
}

/// Reads a staged picker import on the caller's blocking queue, then removes the copy.
///
/// The relative path must name the file returned by the picker. The reader can stream
/// from the file and is called while the staged copy still exists.
pub fn consume_import<T, E>(
    paths: &AppPaths,
    relative_path: &str,
    read: impl FnOnce(&Path, fs::File) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<FileError>,
{
    let mut parts = files::relative_path(relative_path)?.split('/');
    let (Some("Imports"), Some(id), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(FileError::outside(relative_path).into());
    };
    if id.len() != 36
        || !id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(FileError::outside(relative_path).into());
    }
    let base = fs::canonicalize(root(paths, Scope::Documents)?).map_err(FileError::from)?;
    let path = files::resolve(&base, relative_path)?;
    if fs::canonicalize(&path).map_err(FileError::from)? != path {
        return Err(FileError::outside(relative_path).into());
    }
    let cleanup = ImportCleanup(&path);
    let file = files::open_file(&path)?;
    let result = read(&path, file);
    drop(cleanup);
    result
}

struct ImportCleanup<'a>(&'a Path);

impl Drop for ImportCleanup<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
        if let Some(directory) = self.0.parent() {
            let _ = fs::remove_dir(directory);
        }
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    #[test]
    fn consumes_only_a_staged_import_and_cleans_up_after_an_error() -> Result<(), FileError> {
        let root = tempfile::tempdir()?;
        let relative = "Imports/CC886D74-7CB5-4E70-BF0E-75C39AD05FD7/session.pftrace";
        let staged = root.path().join(relative);
        fs::create_dir_all(files::parent(&staged)?)?;
        fs::write(&staged, b"trace")?;
        let paths = AppPaths::new(Some(root.path().to_owned()), None, None);
        let result = consume_import(&paths, relative, |_, mut file| {
            let mut header = [0; 5];
            file.read_exact(&mut header)?;
            assert_eq!(&header, b"trace");
            Err::<(), _>(FileError::new("io", "parser failed"))
        });
        assert!(result.is_err());
        assert!(!files::parent(&staged)?.exists());
        assert!(
            consume_import(&paths, "Imports/../private.txt", |_, _| Ok::<_, FileError>(
                ()
            ))
            .is_err()
        );
        Ok(())
    }
}
