use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use impasto::{Bytes, CancelToken, DiagnosticsSink, ImpastoType, PluginContext, TypedSink};
use impasto_file_system::{Scope, resolve_path};

use crate::{
    HttpError, NativeHttpEvent,
    options::error,
    trace,
    wire::{HttpClient, HttpPiece, HttpRequest, ResponseSink},
};

#[derive(ImpastoType)]
pub(crate) struct DownloadOptions {
    destination_path: String,
    scope: Option<Scope>,
    resume: Option<bool>,
    overwrite: Option<bool>,
}
#[derive(ImpastoType)]
pub(crate) struct UploadOptions {
    path: String,
    scope: Option<Scope>,
    content_type: Option<String>,
}

const WAIT: Duration = Duration::from_secs(30);
const PROGRESS_BYTES: u64 = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) fn upload(
    ctx: &PluginContext,
    request: &mut HttpRequest,
    options: UploadOptions,
) -> Result<(), HttpError> {
    if request.method.eq_ignore_ascii_case("GET")
        || request.method.eq_ignore_ascii_case("HEAD")
        || !request.body.is_empty()
    {
        return Err(error(
            "bad-url",
            "HTTP method or request body conflicts with a file upload",
        ));
    }
    let path = scoped(ctx, options.scope.unwrap_or_default(), &options.path)?;
    let metadata = path
        .metadata()
        .map_err(|_| error("io", "Upload file is unavailable"))?;
    if !metadata.is_file() {
        return Err(error("io", "Upload path must be a regular file"));
    }
    if let Some(content_type) = options.content_type {
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("content-type"));
        request.headers.push(("content-type".into(), content_type));
    }
    request.extra.upload = Some(path);
    request.extra.progress = true;
    Ok(())
}

fn scoped(ctx: &PluginContext, scope: Scope, path: &str) -> Result<PathBuf, HttpError> {
    resolve_path(ctx, scope, path)
        .map_err(|_| error("outside-scope", "HTTP file path is outside its scope"))
}

pub(crate) fn stream(
    transport: &dyn HttpClient,
    mut request: HttpRequest,
    mut sink: TypedSink<NativeHttpEvent>,
    diagnostics: Option<&Arc<dyn DiagnosticsSink>>,
) -> Result<(), HttpError> {
    let limit = sink
        .max_piece_bytes()
        .saturating_sub(128)
        .clamp(1, 64 * 1024);
    let upload_size = request
        .extra
        .upload
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map_or(request.body.len() as u64, |metadata| metadata.len());
    let upload_progress = request.extra.progress && upload_size > 0;
    let mut progress = Progress::new(0);
    if upload_progress {
        send(
            &mut sink,
            NativeHttpEvent::UploadProgress {
                loaded: 0,
                total: Some(upload_size),
            },
        )?;
    }
    let sink = Arc::new(Mutex::new(sink));
    #[cfg(not(target_os = "ios"))]
    if upload_progress && request.extra.upload.is_some() {
        let sink = Arc::clone(&sink);
        request.extra.upload_reporter = Some(Arc::new(move |loaded, total| {
            if loaded < total
                && let Ok(mut sink) = sink.try_lock()
            {
                let _ = sink.try_send(NativeHttpEvent::UploadProgress {
                    loaded,
                    total: Some(total),
                });
            }
        }));
    }
    let mut trace = trace::start(diagnostics, &mut request, "fetch");
    let mut delivery_error = None;
    let result = transport.stream(&request, &mut |piece| {
        trace::piece(&mut trace, &piece);
        let Ok(mut sink) = sink.lock() else {
            return false;
        };
        let delivered = match piece {
            HttpPiece::Head {
                status,
                url,
                redirected,
                headers,
            } => {
                progress.total =
                    header(headers, "content-length").and_then(|value| value.parse().ok());
                send(
                    &mut sink,
                    ResponseSink::head(status, url, redirected, headers),
                )
                .and_then(|_| {
                    if upload_progress && !progress.upload_received {
                        send(
                            &mut sink,
                            NativeHttpEvent::UploadProgress {
                                loaded: upload_size,
                                total: Some(upload_size),
                            },
                        )?;
                    }
                    if request.extra.progress {
                        progress.emit(&mut sink, true)?;
                    }
                    Ok(())
                })
            }
            #[cfg(target_os = "ios")]
            HttpPiece::Upload { loaded, total } => {
                progress.upload_received = true;
                if request.extra.progress {
                    send(&mut sink, NativeHttpEvent::UploadProgress { loaded, total })
                } else {
                    Ok(())
                }
            }
            HttpPiece::Chunk(bytes, end) => {
                progress.loaded += bytes.len() as u64;
                let chunks = bytes.chunks(limit).try_for_each(|piece| {
                    send(
                        &mut sink,
                        NativeHttpEvent::Chunk {
                            data: Bytes(piece.to_vec()),
                        },
                    )
                });
                chunks.and_then(|_| {
                    if request.extra.progress {
                        progress.emit(&mut sink, end)
                    } else {
                        Ok(())
                    }
                })
            }
        };
        if let Err(error) = delivered {
            delivery_error = Some(error);
            false
        } else {
            true
        }
    });
    let result = delivery_error.map_or_else(|| result.map_err(Into::into), Err);
    trace::finish_transfer(trace, &result);
    result
}

struct Progress {
    loaded: u64,
    total: Option<u64>,
    previous: u64,
    last: Instant,
    upload_received: bool,
}
impl Progress {
    fn new(offset: u64) -> Self {
        Self {
            loaded: offset,
            total: None,
            previous: offset,
            last: Instant::now(),
            upload_received: false,
        }
    }
    fn emit(
        &mut self,
        sink: &mut TypedSink<NativeHttpEvent>,
        force: bool,
    ) -> Result<(), HttpError> {
        if force
            || self.loaded.saturating_sub(self.previous) >= PROGRESS_BYTES
            || self.last.elapsed() >= PROGRESS_INTERVAL
        {
            self.previous = self.loaded;
            self.last = Instant::now();
            send(
                sink,
                NativeHttpEvent::DownloadProgress {
                    loaded: self.loaded,
                    total: self.total,
                },
            )?;
        }
        Ok(())
    }
}

fn send(sink: &mut TypedSink<NativeHttpEvent>, event: NativeHttpEvent) -> Result<(), HttpError> {
    sink.send_blocking(event, WAIT)
        .map_err(|_| error("canceled", "AbortError"))
}
fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

struct Destination(PathBuf);
static DOWNLOADS: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();

impl Destination {
    fn reserve(path: PathBuf) -> Result<Self, HttpError> {
        let mut live = DOWNLOADS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| error("io", "Download destination is unavailable"))?;
        if !live.insert(path.clone()) {
            return Err(error("io", "A download already owns this destination"));
        }
        Ok(Self(path))
    }
}
impl Drop for Destination {
    fn drop(&mut self) {
        if let Some(live) = DOWNLOADS.get()
            && let Ok(mut live) = live.lock()
        {
            live.remove(&self.0);
        }
    }
}
struct Partial {
    file: File,
    path: PathBuf,
    validator: PathBuf,
    retain: bool,
    temporary: Option<tempfile::TempPath>,
}
impl Drop for Partial {
    fn drop(&mut self) {
        if !self.retain && self.temporary.is_none() {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(&self.validator);
        }
    }
}

#[derive(Deserialize, Serialize)]
struct ResumeValidator {
    requested: String,
    received: String,
    value: String,
}

fn url_identity(url: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(url))
}

pub(crate) fn download(
    ctx: &PluginContext,
    transport: &dyn HttpClient,
    mut request: HttpRequest,
    options: DownloadOptions,
    mut sink: TypedSink<NativeHttpEvent>,
    diagnostics: Option<&Arc<dyn DiagnosticsSink>>,
) -> Result<(), HttpError> {
    let scope = options.scope.unwrap_or(Scope::Cache);
    let destination = Destination::reserve(scoped(ctx, scope, &options.destination_path)?)?;
    let path = &destination.0;
    let overwrite = options.overwrite.unwrap_or(true);
    if !overwrite && path.exists() {
        return Err(error("io", "Download destination already exists"));
    }
    let root = path
        .parent()
        .ok_or_else(|| error("io", "Download directory is unavailable"))?;
    fs::create_dir_all(root).map_err(io_error)?;
    let keep = options.resume.unwrap_or(false);
    let (file, partial, validator, temporary) = if keep {
        let partial = scoped(ctx, scope, &format!("{}.part", options.destination_path))?;
        let validator = scoped(
            ctx,
            scope,
            &format!("{}.part.validator", options.destination_path),
        )?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&partial)
            .map_err(io_error)?;
        (file, partial, validator, None)
    } else {
        let temp = tempfile::Builder::new()
            .suffix(".part")
            .tempfile_in(root)
            .map_err(io_error)?;
        let (file, temporary) = temp.into_parts();
        (
            file,
            temporary.to_path_buf(),
            PathBuf::new(),
            Some(temporary),
        )
    };
    let mut partial = Partial {
        file,
        path: partial,
        validator,
        retain: keep,
        temporary,
    };
    let mut offset = partial.file.metadata().map_err(io_error)?.len();
    let requested = url_identity(&request.url);
    request.headers.retain(|(name, _)| {
        !(name.eq_ignore_ascii_case("range") || name.eq_ignore_ascii_case("if-range"))
    });
    let validator = if keep && offset > 0 {
        File::open(&partial.validator)
            .ok()
            .and_then(|file| serde_json::from_reader::<_, ResumeValidator>(file.take(4096)).ok())
            .filter(|validator| {
                validator.requested == requested
                    && !validator.value.is_empty()
                    && validator.value.len() < 1024
            })
    } else {
        None
    };
    if let Some(validator) = &validator {
        request
            .headers
            .push(("range".into(), format!("bytes={offset}-")));
        request
            .headers
            .push(("if-range".into(), validator.value.clone()));
    } else {
        offset = 0;
    }
    request.max_body_bytes = Some(usize::MAX);
    request
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("accept-encoding"));
    request
        .headers
        .push(("accept-encoding".into(), "identity".into()));
    let mut progress = Progress::new(offset);
    let mut saved = false;
    let mut resumed = false;
    let mut expected = None;
    let mut trace = trace::start(diagnostics, &mut request, "fetch");
    let mut delivery_error = None;
    let transferred = transport.stream(&request, &mut |piece| {
        trace::piece(&mut trace, &piece);
        let deliver = (|| {
            match piece {
                HttpPiece::Head {
                    status,
                    url,
                    redirected,
                    headers,
                } => {
                    send(
                        &mut sink,
                        ResponseSink::head(status, url, redirected, headers),
                    )?;
                    if !(200..300).contains(&status) {
                        return Ok(());
                    }
                    if status == 206 {
                        expected = header(headers, "content-range")
                            .and_then(|value| range_size(value, offset));
                        if offset == 0
                            || expected.is_none()
                            || validator
                                .as_ref()
                                .is_none_or(|token| token.received != url_identity(url))
                        {
                            partial.retain = false;
                            return Err(error("io", "Server returned an unexpected byte range"));
                        }
                        resumed = true;
                        use std::io::{Seek as _, SeekFrom};
                        partial
                            .file
                            .seek(SeekFrom::Start(offset))
                            .map_err(io_error)?;
                    } else {
                        progress.loaded = 0;
                        partial.file.set_len(0).map_err(io_error)?;
                        if keep {
                            let next = header(headers, "etag")
                                .filter(|value| !value.starts_with("W/"))
                                .or_else(|| header(headers, "last-modified"))
                                .filter(|value| value.len() < 1024);
                            if let Some(next) = next {
                                let mut metadata =
                                    tempfile::NamedTempFile::new_in(root).map_err(io_error)?;
                                let validator = ResumeValidator {
                                    requested: requested.clone(),
                                    received: url_identity(url),
                                    value: next.to_owned(),
                                };
                                serde_json::to_writer(metadata.as_file_mut(), &validator).map_err(
                                    |_| error("io", "Cannot store download resume metadata"),
                                )?;
                                metadata
                                    .persist(&partial.validator)
                                    .map_err(|failure| io_error(failure.error))?;
                            } else {
                                partial.retain = false;
                                let _ = fs::remove_file(&partial.validator);
                            }
                        }
                    }
                    progress.total = expected.or_else(|| {
                        header(headers, "content-length")
                            .and_then(|value| value.parse::<u64>().ok())
                            .map(|length| progress.loaded.saturating_add(length))
                    });
                    saved = true;
                    progress.emit(&mut sink, true)?;
                }
                HttpPiece::Chunk(bytes, end) if saved => {
                    progress.loaded = progress.loaded.saturating_add(bytes.len() as u64);
                    if expected.is_some_and(|size| progress.loaded > size) {
                        partial.retain = false;
                        return Err(error("io", "Server exceeded the promised byte range"));
                    }
                    partial.file.write_all(bytes).map_err(io_error)?;
                    progress.emit(&mut sink, end)?;
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = deliver {
            delivery_error = Some(error);
            false
        } else {
            true
        }
    });
    let outcome = delivery_error.map_or_else(|| transferred.map_err(HttpError::from), Err);
    trace::finish_transfer(trace, &outcome);
    outcome?;
    if !saved {
        return Ok(());
    }
    check_cancel(&request.cancel)?;
    if expected.is_some_and(|size| progress.loaded != size) {
        return Err(error("io", "Server ended before the promised byte range"));
    }
    partial.file.flush().map_err(io_error)?;
    partial.file.sync_all().map_err(io_error)?;
    if overwrite {
        fs::rename(&partial.path, path).map_err(io_error)?;
    } else {
        fs::hard_link(&partial.path, path).map_err(io_error)?;
        let _ = fs::remove_file(&partial.path);
    }
    partial.retain = false;
    if keep {
        let _ = fs::remove_file(&partial.validator);
    }
    send(
        &mut sink,
        NativeHttpEvent::File {
            bytes_written: progress.loaded,
            resumed,
            path: options.destination_path,
        },
    )
}
fn range_size(value: &str, offset: u64) -> Option<u64> {
    value
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .and_then(|(range, size)| {
            let (start, end) = range.split_once('-')?;
            Some((
                start.parse::<u64>().ok()?,
                end.parse::<u64>().ok()?,
                size.parse::<u64>().ok()?,
            ))
        })
        .and_then(|(start, end, size)| {
            (start == offset && end >= offset && end.checked_add(1) == Some(size)).then_some(size)
        })
}
fn io_error(_: std::io::Error) -> HttpError {
    error("io", "HTTP file transfer could not access the filesystem")
}
fn check_cancel(cancel: &CancelToken) -> Result<(), HttpError> {
    if cancel.is_cancelled() {
        Err(error("canceled", "AbortError"))
    } else {
        Ok(())
    }
}
