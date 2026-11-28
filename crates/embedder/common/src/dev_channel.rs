//! Debug-only development connections. Socket I/O and asset writes stay off the UI queue.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dev_protocol::{
    ErrorOverlay, Frame, Hello, HelloAck, Kind, LogBatch, LogEntry, Receipt, VERSION,
};
use engine::tree::{DevErrorCategory, DevErrorOverlay};
use subtle::ConstantTimeEq;

use crate::{Command, Poster};

const LOG_CAPACITY: usize = 256;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const UI_TIMEOUT: Duration = Duration::from_secs(8);
const HEARTBEAT: Duration = Duration::from_secs(5);

static ACTIVE: OnceLock<Arc<Development>> = OnceLock::new();

/// Debug session shared by the transport, startup log sink and asset readers.
pub struct Development {
    token: String,
    assets: PathBuf,
    log: Mutex<LogState>,
    dropped: AtomicU64,
    logging: Logging,
    file_active: AtomicBool,
    last_file: AtomicU64,
    process: u64,
}

#[derive(Default)]
struct LogState {
    sequence: u64,
    history: VecDeque<LogEntry>,
    writer: Option<mpsc::SyncSender<Outbound>>,
}

enum Outbound {
    Frame(Frame),
    Log(LogEntry),
}

/// Returns the active CLI session, if this process was launched for development.
pub fn current() -> Option<&'static Arc<Development>> {
    ACTIVE.get()
}

/// Controls which records reach the development terminal and whether to tee them.
#[derive(Clone, Copy, Debug)]
pub struct Logging {
    pub verbose: bool,
    pub terminal: bool,
}

/// Prepares the process log buffer and discards overlays from previous launches.
pub fn prepare(token: String, caches: &Path, logging: Logging) -> io::Result<Arc<Development>> {
    if !dev_protocol::valid_token(&token) {
        return Err(invalid("invalid development session token"));
    }
    if let Some(prior) = ACTIVE.get() {
        return if bool::from(prior.token.as_bytes().ct_eq(token.as_bytes())) {
            Ok(Arc::clone(prior))
        } else {
            Err(invalid(
                "development session already active in this process",
            ))
        };
    }
    let assets = caches.join("impasto-dev-assets");
    match fs::remove_dir_all(&assets) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let development = Arc::new(Development {
        token,
        assets,
        log: Mutex::default(),
        dropped: AtomicU64::new(0),
        logging,
        file_active: AtomicBool::new(false),
        last_file: AtomicU64::new(0),
        process: nonce()?,
    });
    ACTIVE
        .set(Arc::clone(&development))
        .map_err(|_| invalid("development session already active"))?;
    Ok(development)
}

/// Result of looking up one path relative to the application's asset directory.
pub enum AssetOverride {
    Packaged,
    File(PathBuf),
    Removed,
}

/// Resolves a replacement or tombstone before a platform opens its packaged asset.
pub fn asset_override(relative: &str) -> AssetOverride {
    let Some(dev) = current() else {
        return AssetOverride::Packaged;
    };
    if dev_protocol::validate_asset_path(relative).is_err() {
        return AssetOverride::Packaged;
    }
    dev.asset_override(relative)
}

impl Development {
    fn asset_override(&self, relative: &str) -> AssetOverride {
        if self.assets.join("removed").join(relative).is_file() {
            return AssetOverride::Removed;
        }
        let path = self.assets.join("files").join(relative);
        if path.is_file() {
            AssetOverride::File(path)
        } else {
            AssetOverride::Packaged
        }
    }

    pub(crate) fn suppress_console(&self) -> bool {
        self.logging.terminal
    }

    pub(crate) fn accepts(&self, record: &crate::log_sub::LogRecord) -> bool {
        let level = record.level;
        (self.logging.verbose && level <= tracing::Level::DEBUG)
            || record.target.starts_with("impasto::script")
            || level <= tracing::Level::WARN
    }

    pub(crate) fn record(&self, record: &crate::log_sub::LogRecord) {
        let Ok(mut log) = self.log.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        log.sequence += 1;
        let entry = LogEntry {
            seq: log.sequence,
            ts_ms: millis(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default(),
            ),
            level: record.level.as_str().to_ascii_lowercase(),
            target: record.target.to_owned(),
            message: crate::log_sub::message_with_fields(record)
                .chars()
                .take(8192)
                .collect(),
            frame: record.frame,
        };
        if log.history.len() == LOG_CAPACITY {
            log.history.pop_front();
            if log.writer.is_none() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        log.history.push_back(entry.clone());
        if let Some(writer) = &log.writer
            && writer.try_send(Outbound::Log(entry)).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn serve<S: Socket>(
        &self,
        mut reader: S,
        poster: &Poster,
        identity: &Identity,
        reverse: bool,
    ) -> io::Result<()> {
        reader.set_timeouts(Some(IO_TIMEOUT))?;
        if reverse {
            Frame::json(
                Kind::Hello,
                0,
                &Hello {
                    token: &self.token,
                    protocol: VERSION,
                    cli_version: "",
                },
            )?
            .write(&mut reader)?;
        }
        let hello = Frame::read(&mut reader)?;
        self.authenticate(&hello)?;
        let ack = HelloAck {
            ok: true,
            platform: std::env::consts::OS.to_owned(),
            app_id: identity.app_id.clone(),
            process: self.process,
            instance: identity.instance,
            capabilities: ["hmr", "logs", "assets", "redbox"]
                .map(str::to_owned)
                .into(),
        };
        Frame::json(Kind::HelloAck, 0, &ack)?.write(&mut reader)?;
        reader.set_timeouts(Some(Duration::from_secs(25)))?;

        let writer = reader.duplicate()?;
        let (outbound, pending) = mpsc::sync_channel(LOG_CAPACITY);
        let history = {
            let mut log = lock(&self.log);
            log.writer = Some(outbound.clone());
            log.history.iter().cloned().collect::<Vec<_>>()
        };
        thread::scope(|scope| {
            let writer = scope.spawn(move || {
                let mut writer = writer;
                let result = self.write_frames(&mut writer, pending, history);
                writer.close();
                result
            });
            let result = (|| {
                loop {
                    let frame = Frame::read(&mut reader)?;
                    let response = match frame.kind {
                        Kind::Pong => continue,
                        _ => self.exchange(frame, poster)?,
                    };
                    outbound
                        .send(Outbound::Frame(response))
                        .map_err(io::Error::other)?;
                }
            })();
            lock(&self.log).writer.take();
            drop(outbound);
            reader.close();
            let _ = writer
                .join()
                .map_err(|_| io::Error::other("development writer panicked"))?;
            result
        })
    }

    fn write_frames(
        &self,
        output: &mut impl Write,
        pending: mpsc::Receiver<Outbound>,
        history: Vec<LogEntry>,
    ) -> io::Result<()> {
        if !history.is_empty() {
            self.write_logs(output, history)?;
        }
        let mut heartbeat = Instant::now() + HEARTBEAT;
        let mut next = None;
        loop {
            let now = Instant::now();
            if now >= heartbeat {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                Frame {
                    kind: Kind::Ping,
                    seq: 0,
                    payload: millis(timestamp).to_be_bytes().to_vec(),
                }
                .write(output)?;
                heartbeat = now + HEARTBEAT;
            }
            let message = if let Some(next) = next.take() {
                next
            } else {
                match pending.recv_timeout(heartbeat.saturating_duration_since(Instant::now())) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                }
            };
            match message {
                Outbound::Frame(frame) => frame.write(output)?,
                Outbound::Log(entry) => {
                    let mut entries = vec![entry];
                    while entries.len() < 32 {
                        match pending.try_recv() {
                            Ok(Outbound::Log(entry)) => entries.push(entry),
                            other => {
                                next = other.ok();
                                break;
                            }
                        }
                    }
                    self.write_logs(output, entries)?;
                }
            }
        }
    }

    fn write_logs(&self, writer: &mut impl Write, entries: Vec<LogEntry>) -> io::Result<()> {
        let dropped = self.dropped.swap(0, Ordering::Relaxed);
        Frame::json(Kind::LogBatch, 0, &LogBatch { entries, dropped })?.write(writer)
    }

    fn authenticate(&self, frame: &Frame) -> io::Result<()> {
        let hello: Hello<'_> = frame.decode()?;
        if frame.kind != Kind::Hello
            || frame.seq != 0
            || hello.protocol != VERSION
            || !dev_protocol::valid_token(hello.token)
            || !bool::from(self.token.as_bytes().ct_eq(hello.token.as_bytes()))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "invalid development handshake",
            ));
        }
        Ok(())
    }

    /// Submits a request copied into the application's private development directory.
    pub fn serve_file(self: &Arc<Self>, poster: Poster, directory: PathBuf, revision: u64) {
        if self.file_active.swap(true, Ordering::AcqRel) {
            return;
        }
        let dev = Arc::clone(self);
        let worker = thread::Builder::new()
            .name("impasto-development-file".into())
            .spawn(move || {
                let request = directory.join(format!("request-{revision}"));
                let response = directory.join(format!("reply-{revision}"));
                let reply = (|| {
                    let file = fs::File::open(&request)?;
                    let max = dev_protocol::MAX_PAYLOAD + 1024;
                    let mut bytes = Vec::new();
                    file.take((max + 1) as u64).read_to_end(&mut bytes)?;
                    if bytes.len() > max {
                        return Err(invalid("development request exceeds the file size limit"));
                    }
                    dev.exchange_file(&bytes, &poster)
                })();
                if let Ok(bytes) = reply {
                    let previous = dev.last_file.swap(revision, Ordering::Relaxed);
                    if previous != 0 && previous != revision {
                        let _ = remove(&directory.join(format!("reply-{previous}")));
                    }
                    let _ = remove(&request);
                    let _ = remove(&directory.join(format!("ready-{revision}")));
                    dev.file_active.store(false, Ordering::Release);
                    let _ = atomic_write(&response, &bytes);
                } else {
                    dev.file_active.store(false, Ordering::Release);
                }
            });
        if worker.is_err() {
            self.file_active.store(false, Ordering::Release);
        }
    }

    fn exchange_file(&self, mut request: &[u8], poster: &Poster) -> io::Result<Vec<u8>> {
        self.authenticate(&Frame::read(&mut request)?)?;
        let frame = Frame::read(&mut request)?;
        if !request.is_empty() {
            return Err(invalid("trailing data in development request"));
        }
        let mut response = Vec::new();
        let history = lock(&self.log).history.iter().cloned().collect::<Vec<_>>();
        if !history.is_empty() {
            self.write_logs(&mut response, history)?;
        }
        self.exchange(frame, poster)?.write(&mut response)?;
        Ok(response)
    }

    fn exchange(&self, frame: Frame, poster: &Poster) -> io::Result<Frame> {
        let revision = frame.seq;
        let started = Instant::now();
        let (kind, action) = match frame.kind {
            Kind::SourceUpdate => {
                let (patch, code) = frame.decode_source()?;
                (
                    Kind::UpdateReceipt,
                    Action::Source {
                        patch,
                        code: code.to_owned(),
                    },
                )
            }
            Kind::AssetUpdate | Kind::AssetRemove => {
                let result = frame.decode_asset().and_then(|(path, bytes)| {
                    let font = path.ends_with(".otf") || path.ends_with(".ttf");
                    if font
                        && bytes.is_some_and(|bytes| bytes.len() > engine::text::MAX_APP_FONT_BYTES)
                    {
                        return Err(invalid("font asset exceeds the runtime size limit"));
                    }
                    self.write_asset(path, bytes)?;
                    Ok(if font {
                        Action::Font {
                            path: path.to_owned(),
                            bytes: bytes.map(Arc::from),
                        }
                    } else {
                        Action::Image(path.to_owned())
                    })
                });
                match result {
                    Ok(action) => (Kind::AssetReceipt, action),
                    Err(error) => {
                        return Frame::json(
                            Kind::AssetReceipt,
                            revision,
                            &Receipt::new(
                                revision,
                                millis(started.elapsed()),
                                Err(error.to_string()),
                            ),
                        );
                    }
                }
            }
            Kind::ErrorOverlayShow => (Kind::Pong, Action::Overlay(Some(overlay(frame.decode()?)))),
            Kind::ErrorOverlayClear if frame.payload.is_empty() => {
                (Kind::Pong, Action::Overlay(None))
            }
            Kind::Ping if frame.payload.len() == 8 => {
                return Ok(Frame {
                    kind: Kind::Pong,
                    ..frame
                });
            }
            _ => return Err(invalid("unexpected development request")),
        };

        let (reply, result) = mpsc::sync_channel(1);
        let result = if poster.post_development(Command::Development { action, reply }) {
            result
                .recv_timeout(UI_TIMEOUT)
                .unwrap_or_else(|error| Err(error.to_string()))
        } else {
            Err("the UI session is unavailable".to_owned())
        };
        if kind == Kind::Pong && result.is_ok() {
            return Ok(Frame {
                kind,
                seq: revision,
                payload: Vec::new(),
            });
        }
        Frame::json(
            kind,
            revision,
            &Receipt::new(revision, millis(started.elapsed()), result),
        )
    }

    fn write_asset(&self, relative: &str, bytes: Option<&[u8]>) -> io::Result<()> {
        let data = self.assets.join("files").join(relative);
        let removed = self.assets.join("removed").join(relative);
        if let Some(bytes) = bytes {
            atomic_write(&data, bytes)?;
            remove(&removed)
        } else {
            atomic_write(&removed, &[])?;
            remove(&data)
        }
    }
}

/// Work accepted by the UI queue after authentication.
pub enum Action {
    Source {
        patch: bool,
        code: String,
    },
    Image(String),
    Font {
        path: String,
        bytes: Option<Arc<[u8]>>,
    },
    Overlay(Option<DevErrorOverlay>),
}

fn overlay(error: ErrorOverlay) -> DevErrorOverlay {
    let location = error.file.map(|file| match (error.line, error.column) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        (Some(line), None) => format!("{file}:{line}"),
        _ => file,
    });
    let category = match error.source.as_str() {
        "linker" => DevErrorCategory::Linker,
        "native" => DevErrorCategory::Native,
        _ => DevErrorCategory::Build,
    };
    DevErrorOverlay {
        category,
        title: error.title,
        message: error.message,
        location,
        stack: Vec::new(),
        snippet: error.snippet,
        collapsed: false,
    }
}

pub(crate) fn runtime_overlay(
    category: DevErrorCategory,
    title: &str,
    message: &str,
    stack: &str,
) -> DevErrorOverlay {
    DevErrorOverlay {
        category,
        title: title.to_owned(),
        message: message.to_owned(),
        location: None,
        stack: stack.lines().take(24).map(str::to_owned).collect(),
        snippet: None,
        collapsed: false,
    }
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("invalid file destination"))?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

trait Socket: Read + Write + Send + 'static {
    fn duplicate(&self) -> io::Result<Self>
    where
        Self: Sized;
    fn close(&self);
    fn set_timeouts(&self, read: Option<Duration>) -> io::Result<()>;
}

macro_rules! socket {
    ($type:ty) => {
        impl Socket for $type {
            fn duplicate(&self) -> io::Result<Self> {
                self.try_clone()
            }
            fn close(&self) {
                let _ = self.shutdown(Shutdown::Both);
            }
            fn set_timeouts(&self, read: Option<Duration>) -> io::Result<()> {
                self.set_read_timeout(read)?;
                self.set_write_timeout(Some(IO_TIMEOUT))
            }
        }
    };
}

socket!(TcpStream);
#[cfg(unix)]
socket!(UnixStream);

type Close = Box<dyn FnOnce() + Send>;

struct Identity {
    app_id: String,
    instance: u64,
}

fn nonce() -> io::Result<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(u64::from_ne_bytes(bytes))
}

/// Owns a development listener or reverse connection for one host session.
pub struct Server {
    stopped: Arc<AtomicBool>,
    peer: Arc<Mutex<Option<Close>>>,
    wake: Option<Close>,
}

impl Server {
    #[cfg(unix)]
    pub fn connect_unix(
        dev: Arc<Development>,
        poster: Poster,
        app_id: String,
        path: PathBuf,
    ) -> io::Result<Self> {
        Self::start(
            dev,
            poster,
            app_id,
            move || UnixStream::connect(&path),
            None,
        )
    }

    pub fn connect_tcp(
        dev: Arc<Development>,
        poster: Poster,
        app_id: String,
        address: SocketAddrV4,
    ) -> io::Result<Self> {
        if !address.ip().is_loopback() {
            return Err(invalid("development address must be loopback"));
        }
        Self::start(
            dev,
            poster,
            app_id,
            move || {
                let socket = TcpStream::connect_timeout(&address.into(), IO_TIMEOUT)?;
                socket.set_nodelay(true)?;
                Ok(socket)
            },
            None,
        )
    }

    pub fn listen_tcp(
        dev: Arc<Development>,
        poster: Poster,
        app_id: String,
        port: u16,
    ) -> io::Result<(Self, u16)> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))?;
        let address = listener.local_addr()?;
        let wake: Close = Box::new(move || {
            let _ = TcpStream::connect(address);
        });
        let server = Self::start(
            dev,
            poster,
            app_id,
            move || {
                let (stream, _) = listener.accept()?;
                stream.set_nodelay(true)?;
                Ok(stream)
            },
            Some(wake),
        )?;
        Ok((server, address.port()))
    }

    fn start<S: Socket>(
        dev: Arc<Development>,
        poster: Poster,
        app_id: String,
        mut connect: impl FnMut() -> io::Result<S> + Send + 'static,
        wake: Option<Close>,
    ) -> io::Result<Self> {
        let state = Self {
            stopped: Arc::new(AtomicBool::new(false)),
            peer: Arc::default(),
            wake,
        };
        let stopped = Arc::clone(&state.stopped);
        let peer = Arc::clone(&state.peer);
        let reverse = state.wake.is_none();
        let identity = Identity {
            app_id,
            instance: nonce()?,
        };
        thread::Builder::new()
            .name("impasto-development-io".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    if let Ok(stream) = connect()
                        && let Ok(shutdown) = stream.duplicate()
                    {
                        let mut peer_slot = lock(&peer);
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        *peer_slot = Some(Box::new(move || shutdown.close()));
                        drop(peer_slot);
                        let _ = dev.serve(stream, &poster, &identity, reverse);
                        lock(&peer).take();
                    }
                    if reverse {
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            })?;
        Ok(state)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(close) = lock(&self.peer).take() {
            close();
        }
        if let Some(wake) = self.wake.take() {
            wake();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticates_the_session_token_and_tombstones_do_not_fall_back() {
        let directory = tempfile::tempdir().unwrap();
        let token = "abcd".repeat(16);
        let development = Development {
            token: token.clone(),
            assets: directory.path().join("impasto-dev-assets"),
            log: Mutex::default(),
            dropped: AtomicU64::default(),
            logging: Logging {
                terminal: true,
                verbose: false,
            },
            file_active: AtomicBool::default(),
            last_file: AtomicU64::default(),
            process: 1,
        };
        let hello = |token: &str| {
            Frame::json(
                Kind::Hello,
                0,
                &Hello {
                    token,
                    protocol: VERSION,
                    cli_version: "test",
                },
            )
            .unwrap()
        };
        assert!(development.authenticate(&hello(&token)).is_ok());
        assert!(
            development
                .authenticate(&hello(&"abce".repeat(16)))
                .is_err()
        );

        let asset = "images/icon.png";
        assert!(matches!(
            development.asset_override(asset),
            AssetOverride::Packaged
        ));
        development.write_asset(asset, Some(b"one")).unwrap();
        let AssetOverride::File(initial) = development.asset_override(asset) else {
            panic!("the override did not replace the packaged asset");
        };
        assert_eq!(fs::read(initial).unwrap(), b"one");
        development.write_asset(asset, None).unwrap();
        assert!(matches!(
            development.asset_override(asset),
            AssetOverride::Removed
        ));
        development.write_asset(asset, Some(b"two")).unwrap();
        let AssetOverride::File(replaced) = development.asset_override(asset) else {
            panic!("the replacement did not clear the tombstone");
        };
        assert_eq!(fs::read(replaced).unwrap(), b"two");
    }
}
