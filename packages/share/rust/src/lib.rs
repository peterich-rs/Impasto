//! System sharing of text, URLs and safely staged application files.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{Bytes, ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use impasto_file_system::{Scope, resolve_path};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use url::Url;

const STAGING_DIR: &str = "impasto-share";
const RETAIN_FOR: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy, Debug, Deserialize, Eq, ImpastoType, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ShareStatus {
    Success,
    Dismissed,
    Unavailable,
}

#[derive(Debug, Deserialize, ImpastoType)]
pub struct ShareResult {
    status: ShareStatus,
    activity: Option<String>,
}

#[derive(Debug, ImpastoType)]
struct ShareFile {
    name: Option<String>,
    bytes: Option<Bytes>,
    mime_type: Option<String>,
    path: Option<String>,
    scope: Option<Scope>,
}

#[derive(Clone, Copy, Debug, ImpastoType, Serialize)]
struct ShareAnchor {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Default, ImpastoType)]
struct ShareOptions {
    text: Option<String>,
    url: Option<String>,
    title: Option<String>,
    subject: Option<String>,
    files: Option<Vec<ShareFile>>,
    anchor: Option<ShareAnchor>,
}

#[derive(Debug)]
struct ShareError {
    code: &'static str,
    message: String,
}

impl ShareError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn invalid(message: &str) -> Self {
        Self::new("invalid-input", message)
    }
}

impl ImpastoError for ShareError {
    fn scope(&self) -> &'static str {
        "share"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}

impl From<NativePluginError> for ShareError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "host-unavailable" | "cancelled" => "host-unavailable",
            "busy" => "busy",
            _ => "platform",
        };
        Self::new(code, error.message)
    }
}

impl From<io::Error> for ShareError {
    fn from(error: io::Error) -> Self {
        let code = if error.kind() == io::ErrorKind::NotFound {
            "file-not-found"
        } else {
            "platform"
        };
        Self::new(code, error.to_string())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedFile {
    path: PathBuf,
    name: String,
    mime_type: Option<String>,
}

#[derive(Serialize)]
struct SharePayload {
    text: Option<String>,
    url: Option<String>,
    title: Option<String>,
    subject: Option<String>,
    files: Vec<SharedFile>,
    anchor: Option<ShareAnchor>,
}

fn valid_name(name: &str) -> Result<String, ShareError> {
    let mut sanitized = name
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .take(96)
        .collect::<String>();
    sanitized.truncate(sanitized.floor_char_boundary(240));
    if sanitized.is_empty() || matches!(sanitized.as_str(), "." | "..") {
        return Err(ShareError::invalid("each attachment needs a filename"));
    }
    Ok(sanitized)
}

fn stage(
    contents: ShareFile,
    ctx: &PluginContext,
    folder: &Path,
    index: usize,
) -> Result<SharedFile, ShareError> {
    let (name, source) = match (contents.bytes, contents.path) {
        (Some(bytes), None) => {
            let name = contents
                .name
                .as_deref()
                .ok_or_else(|| ShareError::invalid("binary attachments need a filename"))?;
            (valid_name(name)?, Source::Bytes(bytes))
        }
        (None, Some(path)) => {
            let file =
                resolve_path(ctx, contents.scope.unwrap_or_default(), &path).map_err(|error| {
                    let code = match error.code() {
                        "outside-scope" => "outside-scope",
                        "not-found" => "file-not-found",
                        "host-unavailable" => "host-unavailable",
                        _ => "platform",
                    };
                    ShareError::new(code, error.to_string())
                })?;
            let name = contents
                .name
                .as_deref()
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or_default());
            (valid_name(name)?, Source::File(file))
        }
        _ => {
            return Err(ShareError::invalid(
                "each attachment needs either a path or bytes",
            ));
        }
    };
    let entry = folder.join(index.to_string());
    fs::create_dir(&entry)?;
    let path = entry.join(&name);
    let mut output = File::create(&path)?;
    match source {
        Source::Bytes(bytes) => output.write_all(&bytes.0)?,
        Source::File(file) => {
            if !fs::metadata(&file)?.is_file() {
                return Err(ShareError::new(
                    "file-not-found",
                    "attachment is not a regular file",
                ));
            }
            io::copy(&mut File::open(file)?, &mut output)?;
        }
    }
    Ok(SharedFile {
        path,
        name,
        mime_type: contents.mime_type,
    })
}

enum Source {
    Bytes(Bytes),
    File(PathBuf),
}

fn validate(options: &ShareOptions) -> Result<(), ShareError> {
    if options
        .text
        .as_ref()
        .is_none_or(|text| text.trim().is_empty())
        && options.url.as_ref().is_none_or(|url| url.is_empty())
        && options.files.as_ref().is_none_or(Vec::is_empty)
    {
        return Err(ShareError::invalid(
            "share requires text, a URL, or at least one attachment",
        ));
    }
    if let Some(url) = &options.url {
        let parsed =
            Url::parse(url).map_err(|_| ShareError::invalid("share requires an absolute URL"))?;
        if matches!(
            parsed.scheme(),
            "file" | "data" | "javascript" | "content" | "vbscript" | "intent"
        ) || url.chars().any(char::is_control)
        {
            return Err(ShareError::invalid("this URL scheme cannot be shared"));
        }
    }
    if let Some(anchor) = options.anchor
        && (![anchor.x, anchor.y, anchor.width, anchor.height]
            .into_iter()
            .all(f64::is_finite)
            || anchor.width <= 0.0
            || anchor.height <= 0.0)
    {
        return Err(ShareError::invalid(
            "the share anchor needs finite coordinates and positive dimensions",
        ));
    }
    Ok(())
}

fn prepare(
    ctx: &PluginContext,
    options: ShareOptions,
) -> Result<(SharePayload, Option<TempDir>), ShareError> {
    let mut files = Vec::new();
    let folder = if let Some(entries) = options.files.filter(|entries| !entries.is_empty()) {
        let cache = ctx
            .paths()
            .cache_dir()
            .map_err(|error| ShareError::new("host-unavailable", error.to_string()))?;
        let root = cache.join(STAGING_DIR);
        fs::create_dir_all(&root)?;
        let root = fs::canonicalize(&root)?;
        if !root.starts_with(fs::canonicalize(cache)?) {
            return Err(ShareError::new(
                "outside-scope",
                "share cache escapes the application directory",
            ));
        }
        reap(&root)?;
        let folder = tempfile::Builder::new().prefix("share-").tempdir_in(root)?;
        for (index, entry) in entries.into_iter().enumerate() {
            files.push(stage(entry, ctx, folder.path(), index)?);
        }
        Some(folder)
    } else {
        None
    };
    Ok((
        SharePayload {
            text: options.text,
            url: options.url,
            title: options.title,
            subject: options.subject,
            files,
            anchor: options.anchor,
        },
        folder,
    ))
}

fn reap(root: &Path) -> Result<(), ShareError> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("share-")
            && entry.file_type()?.is_dir()
            && entry
                .metadata()?
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|elapsed| elapsed >= RETAIN_FOR)
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[impasto::module(name = "Share", specifier = "@impasto/share/native", init = init)]
struct SharePlugin {
    context: PluginContext,
    native: NativePlugin,
    busy: AtomicBool,
}

impl SharePlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            context: ctx.clone(),
            native: NativePlugin::new(ctx, "@impasto/share"),
            busy: AtomicBool::new(false),
        })
    }
}

#[impasto::export]
impl SharePlugin {
    pub fn is_share_host_available(&self) -> bool {
        self.native.is_hosted()
    }

    #[impasto::method(awaits_user)]
    pub async fn share(&self, options: ShareOptions) -> Result<ShareResult, ShareError> {
        validate(&options)?;
        if !self.native.has_channel() {
            return Err(ShareError::new(
                "host-unavailable",
                "sharing requires a native application host",
            ));
        }
        if self.busy.swap(true, Ordering::Acquire) {
            return Err(ShareError::new("busy", "a share sheet is already open"));
        }
        let _busy = Busy(&self.busy);
        let context = self.context.clone();
        let (payload, folder) = self
            .context
            .runtime()
            .spawn_blocking(move || prepare(&context, options))
            .await
            .map_err(|error| ShareError::new("host-unavailable", error.to_string()))??;
        let response: Result<ShareResult, _> = self.native.request("share", &payload).await;
        if let Some(folder) = folder {
            // Android's selected activity can open a granted URI after the chooser returns.
            let retain = cfg!(target_os = "android")
                && response
                    .as_ref()
                    .is_ok_and(|result| result.status == ShareStatus::Success);
            let _ = self
                .context
                .runtime()
                .spawn_blocking(move || {
                    if retain {
                        let _ = folder.keep();
                    }
                })
                .await;
        }
        Ok(response?)
    }
}

/// Registers the system share bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(SharePlugin::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_keeps_share_attachments_inside_the_session_cache() {
        let sandbox = tempfile::tempdir().unwrap();
        let paths = impasto::AppPaths::isolated(sandbox.path()).unwrap();
        let ctx = PluginContext::new(impasto::Runtime::default(), paths, |_| false);
        let input = ShareFile {
            name: Some("a/b.txt".into()),
            bytes: Some(Bytes(b"test".to_vec())),
            mime_type: None,
            path: None,
            scope: None,
        };
        let (payload, lease) = prepare(
            &ctx,
            ShareOptions {
                files: Some(vec![input]),
                ..ShareOptions::default()
            },
        )
        .unwrap();
        assert_eq!(payload.files[0].name, "a_b.txt");
        assert_eq!(fs::read(&payload.files[0].path).unwrap(), b"test");
        drop(lease);
        assert!(!payload.files[0].path.exists());
    }
}
