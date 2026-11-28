//! Development connection management and the common run event loop.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal, Read, Write};
use std::net::{Shutdown, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use dev_protocol::{ErrorOverlay, Frame, Hello, HelloAck, Kind, LogBatch, Receipt, VERSION};
use subtle::ConstantTimeEq;

use crate::{
    app::App,
    build::{self, BuildOptions},
    watch::{self, ProjectWatch, Scope, Tier},
};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(12);
const RECONNECT_DELAY: Duration = Duration::from_millis(500);

pub(crate) enum Event {
    Files(watch::Batch),
    Warning(String),
    Connected { generation: u64, fresh_host: bool },
    Disconnected,
    ChildExited(u32, ExitStatus),
    RetryFiles,
    Shutdown,
}

/// Installs the process interrupt handler before any managed child is spawned.
pub(crate) fn events() -> Result<(mpsc::Sender<Event>, mpsc::Receiver<Event>)> {
    let (sender, receiver) = mpsc::channel();
    let interrupt = sender.clone();
    ctrlc::set_handler(move || {
        let _ = interrupt.send(Event::Shutdown);
    })?;
    Ok((sender, receiver))
}

pub(crate) fn token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("生成调试会话令牌失败: {error}"))?;
    use std::fmt::Write as _;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        write!(token, "{byte:02x}")?;
    }
    Ok(token)
}

pub(crate) enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Stream {
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Tcp(socket) => socket.try_clone().map(Self::Tcp),
            #[cfg(unix)]
            Self::Unix(socket) => socket.try_clone().map(Self::Unix),
        }
    }

    fn timeout(&self, read: Option<Duration>, write: Duration) -> io::Result<()> {
        match self {
            Self::Tcp(socket) => {
                socket.set_nodelay(true)?;
                socket.set_read_timeout(read)?;
                socket.set_write_timeout(Some(write))
            }
            #[cfg(unix)]
            Self::Unix(socket) => {
                socket.set_read_timeout(read)?;
                socket.set_write_timeout(Some(write))
            }
        }
    }

    fn close(&self) {
        match self {
            Self::Tcp(socket) => {
                let _ = socket.shutdown(Shutdown::Both);
            }
            #[cfg(unix)]
            Self::Unix(socket) => {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(socket) => socket.read(bytes),
            #[cfg(unix)]
            Self::Unix(socket) => socket.read(bytes),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(socket) => socket.write(bytes),
            #[cfg(unix)]
            Self::Unix(socket) => socket.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(socket) => socket.flush(),
            #[cfg(unix)]
            Self::Unix(socket) => socket.flush(),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Peer {
    generation: u64,
    stream: Stream,
    receipt: Option<(u64, Kind, mpsc::SyncSender<Receipt>)>,
}

#[derive(Default)]
struct State {
    stopped: bool,
    peer: Option<Peer>,
}

pub(crate) struct Channel {
    state: Arc<Mutex<State>>,
    wake: Option<Box<dyn FnOnce()>>,
}

impl Channel {
    pub(crate) fn start(
        mut dial: impl FnMut() -> Result<Stream> + Send + 'static,
        wake: Option<Box<dyn FnOnce()>>,
        token: String,
        app_id: String,
        events: mpsc::Sender<Event>,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(State::default()));
        let connection = Arc::clone(&state);
        let reverse = wake.is_some();
        thread::Builder::new()
            .name("impasto-dev-channel".into())
            .spawn(move || {
                static NEXT_PEER: AtomicU64 = AtomicU64::new(1);
                let mut instance = None;
                let mut process = None;
                let mut announced_down = false;
                let mut last_log = 0;
                while !lock(&connection).stopped {
                    let connected = dial().and_then(|mut stream| {
                        stream.timeout(Some(Duration::from_secs(3)), Duration::from_secs(3))?;
                        if reverse {
                            let frame = Frame::read(&mut stream)?;
                            let hello = frame.decode::<Hello<'_>>()?;
                            ensure!(
                                frame.kind == Kind::Hello
                                    && frame.seq == 0
                                    && hello.protocol == VERSION
                                    && dev_protocol::valid_token(hello.token)
                                    && bool::from(hello.token.as_bytes().ct_eq(token.as_bytes())),
                                "反向开发连接认证失败"
                            );
                        }
                        Frame::json(
                            Kind::Hello,
                            0,
                            &Hello {
                                token: &token,
                                protocol: VERSION,
                                cli_version: env!("CARGO_PKG_VERSION"),
                            },
                        )?
                        .write(&mut stream)?;
                        let ack = Frame::read(&mut stream)?;
                        ensure!(
                            ack.kind == Kind::HelloAck && ack.seq == 0,
                            "无效的开发通道握手"
                        );
                        let ack = ack.decode::<HelloAck>()?;
                        ensure!(ack.ok && ack.app_id == app_id, "开发通道应用身份不匹配");
                        stream.timeout(Some(Duration::from_secs(20)), Duration::from_secs(3))?;
                        Ok((stream, ack))
                    });
                    if let Ok((mut stream, ack)) = connected {
                        let Ok(writer) = stream.try_clone() else {
                            break;
                        };
                        let generation = NEXT_PEER.fetch_add(1, Ordering::Relaxed);
                        let fresh_host = instance
                            .replace(ack.instance)
                            .is_some_and(|previous| previous != ack.instance);
                        if process.replace(ack.process) != Some(ack.process) {
                            last_log = 0;
                        }
                        {
                            let mut guard = lock(&connection);
                            if guard.stopped {
                                break;
                            }
                            guard.peer = Some(Peer {
                                generation,
                                stream: writer,
                                receipt: None,
                            });
                        }
                        announced_down = false;
                        if events
                            .send(Event::Connected {
                                generation,
                                fresh_host,
                            })
                            .is_err()
                        {
                            break;
                        }
                        while let Ok(frame) = Frame::read(&mut stream) {
                            match frame.kind {
                                Kind::UpdateReceipt | Kind::AssetReceipt | Kind::Pong
                                    if frame.seq != 0 =>
                                {
                                    let mut guard = lock(&connection);
                                    if let Some(peer) = guard.peer.as_mut()
                                        && peer.receipt.as_ref().is_some_and(|(id, kind, _)| {
                                            *id == frame.seq && *kind == frame.kind
                                        })
                                        && let Some((id, _, reply)) = peer.receipt.take()
                                    {
                                        let receipt = decode_receipt(&frame);
                                        if let Ok(receipt) = receipt
                                            && receipt.revision == id
                                        {
                                            let _ = reply.try_send(receipt);
                                        }
                                    }
                                }
                                Kind::LogBatch => {
                                    if let Ok(batch) = frame.decode::<LogBatch>() {
                                        print_logs(batch, &mut last_log);
                                    }
                                }
                                Kind::Ping => {
                                    let pong = Frame {
                                        kind: Kind::Pong,
                                        ..frame
                                    };
                                    let mut guard = lock(&connection);
                                    if let Some(peer) = guard.peer.as_mut()
                                        && pong.write(&mut peer.stream).is_err()
                                    {
                                        break;
                                    }
                                }
                                _ => break,
                            }
                        }
                        lock(&connection).peer.take();
                    }
                    if !announced_down && !lock(&connection).stopped {
                        announced_down = true;
                        if events.send(Event::Disconnected).is_err() {
                            break;
                        }
                    }
                    thread::sleep(RECONNECT_DELAY);
                }
            })?;
        Ok(Self { state, wake })
    }

    pub(crate) fn generation(&self) -> Option<u64> {
        lock(&self.state).peer.as_ref().map(|peer| peer.generation)
    }

    fn exchange(&self, frame: &Frame, reply: Kind) -> Result<Receipt> {
        let (tx, rx) = mpsc::sync_channel(1);
        let generation = {
            let mut guard = lock(&self.state);
            let peer = guard.peer.as_mut().context("设备调试通道未连接")?;
            peer.receipt = Some((frame.seq, reply, tx));
            if let Err(error) = frame.write(&mut peer.stream) {
                peer.stream.close();
                return Err(error.into());
            }
            peer.generation
        };
        match rx.recv_timeout(RESPONSE_TIMEOUT) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                if let Some(peer) = lock(&self.state).peer.as_ref()
                    && peer.generation == generation
                {
                    peer.stream.close();
                }
                Err(error).context("设备未确认调试请求")
            }
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        state.stopped = true;
        if let Some(peer) = state.peer.take() {
            peer.stream.close();
        }
        drop(state);
        if let Some(wake) = self.wake.take() {
            wake();
        }
    }
}

fn print_logs(batch: LogBatch, last: &mut u64) {
    let stderr = io::stderr();
    let colors = stderr.is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut stderr = stderr.lock();
    if batch.dropped != 0 {
        let _ = writeln!(stderr, "impasto: 设备日志队列丢弃 {} 条记录", batch.dropped);
    }
    for entry in batch.entries {
        if entry.seq <= *last {
            continue;
        }
        *last = entry.seq;
        let js = entry.target.starts_with("impasto::script");
        let label = match (js, entry.level.as_str()) {
            (true, "error") => "JS ERR",
            (true, "warn") => "JS WARN",
            (true, "debug") => "JS DBG",
            (true, _) => "JS",
            (false, "error") => "ERR",
            (false, "warn") => "WARN",
            (false, "debug") => "DBG",
            (false, _) => "INFO",
        };
        let color = if colors {
            match entry.level.as_str() {
                "error" => "\x1b[31m",
                "warn" => "\x1b[33m",
                _ => "\x1b[36m",
            }
        } else {
            ""
        };
        let reset = if colors { "\x1b[0m" } else { "" };
        let message = entry.message.replace('\r', "");
        let mut lines = message.split('\n');
        let first = lines.next().unwrap_or_default();
        if js {
            let _ = writeln!(stderr, "{color}[{label}]{reset} {}", log_text(first));
        } else {
            let _ = writeln!(
                stderr,
                "{color}[{label}]{reset} {} {}",
                log_text(&entry.target),
                log_text(first)
            );
        }
        for line in lines {
            let _ = writeln!(stderr, "  {}", log_text(line));
        }
    }
}

fn log_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\t')
        .collect()
}

#[derive(Debug)]
struct FileTransportUnavailable;

impl std::fmt::Display for FileTransportUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("文件调试通道未确认请求")
    }
}

impl std::error::Error for FileTransportUnavailable {}

#[derive(Debug)]
struct DeviceRejected(String);

impl std::fmt::Display for DeviceRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DeviceRejected {}

pub(crate) enum Delivery {
    Socket(Channel),
    Files(Box<dyn FileDelivery>),
}

pub(crate) trait FileDelivery {
    fn exchange(&mut self, frame: &Frame, reply: Kind) -> Result<Receipt>;
}

impl Delivery {
    fn generation(&self) -> Option<u64> {
        match self {
            Self::Socket(channel) => channel.generation(),
            Self::Files(_) => Some(1),
        }
    }

    fn exchange(&mut self, frame: &Frame, reply: Kind) -> Result<Receipt> {
        match self {
            Self::Socket(channel) => channel.exchange(frame, reply),
            Self::Files(channel) => channel
                .exchange(frame, reply)
                .context(FileTransportUnavailable),
        }
    }
}

pub(crate) fn file_request(token: &str, frame: &Frame) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    Frame::json(
        Kind::Hello,
        0,
        &Hello {
            token,
            protocol: VERSION,
            cli_version: env!("CARGO_PKG_VERSION"),
        },
    )?
    .write(&mut bytes)?;
    frame.write(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn file_receipt(
    mut bytes: &[u8],
    seq: u64,
    kind: Kind,
    last_log: &mut u64,
) -> Result<Receipt> {
    while !bytes.is_empty() {
        let frame = Frame::read(&mut bytes)?;
        if frame.kind == Kind::LogBatch {
            print_logs(frame.decode()?, last_log);
        } else if frame.seq == seq && frame.kind == kind {
            return decode_receipt(&frame).map_err(Into::into);
        }
    }
    bail!("设备未返回调试请求 {seq} 的回执")
}

fn decode_receipt(frame: &Frame) -> io::Result<Receipt> {
    if frame.kind == Kind::Pong && frame.payload.is_empty() {
        Ok(Receipt::new(frame.seq, 0, Ok("acknowledged")))
    } else {
        frame.decode::<Receipt>()
    }
}

pub(crate) struct Running {
    pub(crate) manifest: Option<Vec<u8>>,
    pub(crate) source: Option<String>,
    pub(crate) channel: Delivery,
    pub(crate) child: Option<Child>,
}

pub(crate) struct Child {
    pid: u32,
    alive: Arc<Mutex<bool>>,
}

impl Child {
    pub(crate) fn wait(
        mut child: std::process::Child,
        events: mpsc::Sender<Event>,
    ) -> Result<Self> {
        let pid = child.id();
        let alive = Arc::new(Mutex::new(true));
        let running = Arc::clone(&alive);
        thread::Builder::new()
            .name("impasto-app-wait".into())
            .spawn(move || match child.wait() {
                Ok(status) => {
                    *lock(&running) = false;
                    let _ = events.send(Event::ChildExited(pid, status));
                }
                Err(error) => {
                    *lock(&running) = false;
                    let _ = events.send(Event::Warning(format!("等待应用退出: {error}")));
                }
            })?;
        Ok(Self { pid, alive })
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        #[cfg(unix)]
        if *lock(&self.alive) {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &self.pid.to_string()])
                .status();
        }
    }
}

pub(crate) trait Target {
    fn platform(&self) -> &'static str;
    fn launch(
        &self,
        app: &App,
        options: &BuildOptions,
        events: &mpsc::Sender<Event>,
        attach: bool,
    ) -> Result<Running>;
    fn extra_inputs(&self, _app: &App) -> Result<Vec<PathBuf>> {
        Ok(Vec::new())
    }
}

pub(crate) fn run(
    target: &impl Target,
    mut app: App,
    options: &BuildOptions,
    attach: bool,
) -> Result<()> {
    let (events, rx) = events()?;
    let mut watcher = ProjectWatch::new(
        Scope::app(&app, &target.extra_inputs(&app)?),
        events.clone(),
    )?;
    let mut installed = watcher.snapshot().clone();
    let mut applied = installed.clone();
    let mut live = target.launch(&app, options, &events, attach)?;
    watcher.set_scope(Scope::app(&app, &target.extra_inputs(&app)?))?;
    let mut revision = 0;
    let mut connected_before = false;
    let mut packaged_source = live.source.clone();
    let mut packaged_metadata = ImageMetadata::read(&app, &installed);
    let mut metadata = packaged_metadata.clone();
    let mut overlay = None;
    let mut retry = false;
    let mut retry_scheduled = false;
    eprintln!("impasto: 正在监听应用文件。Ctrl-C 退出");
    while let Ok(event) = rx.recv() {
        let mut reconnected = false;
        match event {
            Event::Files(batch) => match watcher.reconcile(batch) {
                Ok(changes) if !changes.is_empty() => retry = true,
                Ok(_) => continue,
                Err(error) => {
                    eprintln!("impasto: 读取文件变更失败。{error:#}");
                    continue;
                }
            },
            Event::Connected {
                generation,
                fresh_host,
            } if live.channel.generation() == Some(generation) => {
                reconnected = fresh_host;
                if connected_before {
                    eprintln!("impasto: 调试通道已重连");
                }
                connected_before = true;
                if fresh_host {
                    applied = installed.clone();
                    live.source = packaged_source.clone();
                    metadata = packaged_metadata.clone();
                    if let Some(error) = &overlay {
                        let _ = show(&mut live.channel, &mut revision, error);
                    }
                }
                if !retry && !reconnected {
                    continue;
                }
            }
            Event::Connected { .. } => continue,
            Event::Disconnected => {
                if live.channel.generation().is_none() {
                    eprintln!("impasto: 等待设备调试通道重新连接");
                }
                continue;
            }
            Event::ChildExited(pid, status) => {
                if live.child.as_ref().is_some_and(|child| child.pid == pid) {
                    ensure!(status.success(), "应用退出 ({status})");
                    return Ok(());
                }
                continue;
            }
            Event::Warning(message) => {
                eprintln!("impasto: {message}");
                continue;
            }
            Event::RetryFiles => retry_scheduled = false,
            Event::Shutdown => return Ok(()),
        }
        let changes = watch::changes(&applied, watcher.snapshot());
        if changes.is_empty() && !reconnected {
            retry = false;
            continue;
        }
        let tier = changes.iter().map(|path| watch::classify(&app, path)).max();
        let started = Instant::now();
        let result = if tier.is_some_and(|tier| tier >= Tier::NativeRust) {
            let launch = (|| {
                let updated = App::current()?;
                let next = crate::toolchain::with_build_diagnostics(|| {
                    target.launch(&updated, options, &events, false)
                })?;
                watcher.set_scope(Scope::app(&updated, &target.extra_inputs(&updated)?))?;
                app = updated;
                live = next;
                installed = watcher.snapshot().clone();
                connected_before = false;
                packaged_source = live.source.clone();
                packaged_metadata = ImageMetadata::read(&app, &installed);
                metadata = packaged_metadata.clone();
                Ok(())
            })();
            launch.map(|()| {
                eprintln!(
                    "impasto: {:?} rebuild ({}ms)",
                    tier.unwrap_or(Tier::NativeRust),
                    started.elapsed().as_millis()
                )
            })
        } else if live.channel.generation().is_none() {
            continue;
        } else {
            update(
                &mut live,
                &app,
                options,
                target.platform(),
                &changes,
                &mut metadata,
                &mut revision,
            )
        };
        match result {
            Ok(()) => {
                retry = false;
                overlay = None;
                applied = watcher.snapshot().clone();
            }
            Err(error) if error.is::<FileTransportUnavailable>() => {
                if !retry_scheduled {
                    eprintln!("impasto: 等待文件调试通道。{error:#}");
                    retry_scheduled = true;
                    let sender = events.clone();
                    thread::spawn(move || {
                        thread::sleep(RECONNECT_DELAY);
                        let _ = sender.send(Event::RetryFiles);
                    });
                }
            }
            Err(error) => {
                eprintln!("impasto: 更新失败。{error:#}");
                if live.channel.generation().is_some()
                    && (!error.is::<DeviceRejected>() || tier == Some(Tier::AssetOnly))
                {
                    let problem = diagnostic(&app.root, tier, &error);
                    let _ = show(&mut live.channel, &mut revision, &problem);
                    overlay = Some(problem);
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ImageMetadata(BTreeMap<PathBuf, Option<serde_json::Value>>);

impl ImageMetadata {
    fn read(app: &App, snapshot: &watch::Snapshot) -> Self {
        let paths = snapshot
            .keys()
            .filter(|path| path.starts_with(app.assets()))
            .flat_map(|path| metadata_paths(path))
            .collect::<BTreeSet<_>>();
        Self(Self::values(app, paths).collect())
    }

    fn compare(
        &self,
        app: &App,
        paths: BTreeSet<PathBuf>,
    ) -> BTreeMap<PathBuf, Option<serde_json::Value>> {
        Self::values(app, paths)
            .filter(|(path, value)| self.0.get(path).and_then(Option::as_ref) != value.as_ref())
            .collect()
    }

    fn values(
        app: &App,
        paths: BTreeSet<PathBuf>,
    ) -> impl Iterator<Item = (PathBuf, Option<serde_json::Value>)> {
        paths.into_iter().map(move |path| {
            let value = bundler::image_metadata(app.assets(), &path).ok();
            (path, value)
        })
    }
}

fn metadata_paths(path: &Path) -> impl Iterator<Item = PathBuf> {
    let mut paths = Vec::new();
    if let Some(name) = path.file_name().and_then(|name| name.to_str())
        && let Some((stem, extension @ ("png" | "jpg" | "jpeg" | "webp"))) = name.rsplit_once('.')
    {
        paths.push(path.to_owned());
        if let Some(base) = stem
            .strip_suffix("@2x")
            .or_else(|| stem.strip_suffix("@3x"))
        {
            paths.push(path.with_file_name(format!("{base}.{extension}")));
        }
    }
    paths.into_iter()
}

fn update(
    live: &mut Running,
    app: &App,
    options: &BuildOptions,
    platform: &str,
    changes: &[PathBuf],
    metadata: &mut ImageMetadata,
    revision: &mut u64,
) -> Result<()> {
    let started = Instant::now();
    let mut script = Vec::new();
    let mut assets = 0;
    let mut image_changes = BTreeSet::new();
    let mut asset_ms = 0;
    for path in changes {
        if watch::classify(app, path) == Tier::AssetOnly {
            let name = path
                .strip_prefix(app.assets())?
                .to_str()
                .context("资产路径必须是 UTF-8")?
                .replace('\\', "/");
            let bytes = match std::fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            let frame = Frame::asset(next_revision(revision), &name, bytes)?;
            let receipt = live.channel.exchange(&frame, Kind::AssetReceipt)?;
            asset_ms += receipt.apply_ms;
            accepted(receipt)?;
            assets += 1;
            image_changes.extend(metadata_paths(path));
        } else {
            script.push(path.clone());
        }
    }
    let changed_metadata = metadata.compare(app, image_changes);
    script.extend(changed_metadata.keys().cloned());
    if script.is_empty() {
        let frame = Frame {
            kind: Kind::ErrorOverlayClear,
            seq: next_revision(revision),
            payload: Vec::new(),
        };
        accepted(live.channel.exchange(&frame, Kind::Pong)?)?;
        if assets != 0 {
            eprintln!(
                "impasto: Asset update ({assets} files · {}ms)",
                started.elapsed().as_millis()
            );
        }
        return Ok(());
    }
    crate::sync::refresh_development_types(app, &script)?;
    if let Some(linker) = &options.hot {
        linker.borrow_mut().set_changes(Some(script));
    }
    let source = build::bundle_update(app, options, platform, live.manifest.as_deref())
        .context("脚本编译或链接失败")?;
    let build_ms = started
        .elapsed()
        .as_millis()
        .saturating_sub(u128::from(asset_ms));
    let outcome = bundler::delta(live.source.as_deref().unwrap_or_default(), &source);
    let result = match outcome {
        bundler::DeltaOutcome::Patch {
            code,
            changed_modules,
            updated_components,
        } => {
            let frame = Frame::source(next_revision(revision), true, &code)?;
            let receipt = live.channel.exchange(&frame, Kind::UpdateReceipt)?;
            if receipt.ok {
                live.source = Some(source);
                let modules = if changed_modules.is_empty() {
                    "module metadata".to_owned()
                } else {
                    changed_modules.join(", ")
                };
                let names = if updated_components.is_empty() {
                    "module graph".to_owned()
                } else {
                    updated_components.join(", ")
                };
                eprintln!(
                    "impasto: HMR patched {names} ({modules} · build {build_ms}ms · apply {}ms)",
                    receipt.apply_ms
                );
                metadata.0.extend(changed_metadata);
                return Ok(());
            }
            full_reload(
                live,
                source,
                revision,
                build_ms,
                bundler::ReloadReason::RuntimePatchRejected(
                    receipt.error.unwrap_or_else(|| "设备拒绝补丁".to_owned()),
                ),
            )
        }
        bundler::DeltaOutcome::FullReload { reason } => {
            full_reload(live, source, revision, build_ms, reason)
        }
    };
    if result.is_ok() {
        metadata.0.extend(changed_metadata);
    }
    result
}

fn full_reload(
    live: &mut Running,
    source: String,
    revision: &mut u64,
    build_ms: u128,
    reason: bundler::ReloadReason,
) -> Result<()> {
    let frame = Frame::source(next_revision(revision), false, &source)?;
    let receipt = live.channel.exchange(&frame, Kind::UpdateReceipt)?;
    let apply_ms = receipt.apply_ms;
    accepted(receipt)?;
    live.source = Some(source);
    eprintln!("impasto: Full reload ({reason} · build {build_ms}ms · apply {apply_ms}ms)");
    Ok(())
}

fn accepted(receipt: Receipt) -> Result<()> {
    if receipt.ok {
        Ok(())
    } else {
        Err(DeviceRejected(
            receipt
                .error
                .unwrap_or_else(|| "设备拒绝开发更新".to_owned()),
        )
        .into())
    }
}

fn next_revision(revision: &mut u64) -> u64 {
    *revision += 1;
    *revision
}

fn show(channel: &mut Delivery, revision: &mut u64, problem: &ErrorOverlay) -> Result<()> {
    let frame = Frame::json(Kind::ErrorOverlayShow, next_revision(revision), problem)?;
    accepted(channel.exchange(&frame, Kind::Pong)?)
}

fn diagnostic(root: &Path, tier: Option<Tier>, error: &anyhow::Error) -> ErrorOverlay {
    let message = format!("{error:#}");
    let native = tier.is_some_and(|tier| tier >= Tier::NativeRust);
    let location = message.lines().find_map(|line| {
        let line = line
            .trim()
            .trim_start_matches("--> ")
            .trim_start_matches("at ");
        let line = line.split_once('[').map_or(line, |(_, tail)| tail);
        let line = line.trim_end_matches([']', ')']);
        let (rest, column) = line.rsplit_once(':')?;
        let (file, row) = rest.rsplit_once(':')?;
        let row = row.parse::<usize>().ok()?;
        let column = column.parse::<usize>().ok()?;
        let file = file.trim_start_matches('(');
        let path = root.join(file);
        path.is_file()
            .then_some((file.to_owned(), row, column, path))
    });
    let snippet = location.as_ref().and_then(|(_, row, _, path)| {
        let source = std::fs::read_to_string(path).ok()?;
        Some(
            source
                .lines()
                .skip(row.saturating_sub(2))
                .take(3)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    });
    ErrorOverlay {
        source: if native {
            "native"
        } else if message.contains("Rolldown") {
            "linker"
        } else {
            "compiler"
        }
        .into(),
        title: if native {
            "Native build failed"
        } else {
            "Script update failed"
        }
        .into(),
        message,
        file: location.as_ref().map(|(file, ..)| file.clone()),
        line: location.as_ref().map(|(_, row, ..)| *row),
        column: location.map(|(_, _, column, _)| column),
        snippet,
    }
}
