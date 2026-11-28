use std::collections::HashMap;
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use devtools_protocol::{
    BINARY_FLAG_ZSTD, BINARY_HEADER_LEN, BinaryEnvelopeHeader, BinaryMsgType, Channel,
    ChannelCursors, MAX_BINARY_PAYLOAD, PROTOCOL_VERSION_V2, SessionInfo,
};
use devtools_symbols::SymbolTable;
use futures_channel::oneshot;
use impasto::{CancelToken, TypedSink};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::DesktopError;
use crate::discovery::{self, Target, Tunnel};
use crate::store::{CHANNELS, ConnectionState, FrameWindowKey, Recording, Shared};
use crate::types::{SessionOverview, TickSummary};

const READ_TIMEOUT: Duration = Duration::from_millis(25);
const RPC_TIMEOUT: Duration = Duration::from_secs(6);
const TICK_INTERVAL: Duration = Duration::from_millis(100);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(3);
const RETRY_WINDOW: Duration = Duration::from_secs(60);
const PING_INTERVAL: Duration = Duration::from_secs(3);
const MAX_PENDING: usize = 32;

pub(crate) const DEFAULT_CHANNELS: [Channel; 4] = [
    Channel::Frames,
    Channel::Network,
    Channel::Logs,
    Channel::Errors,
];

type Reply<T> = oneshot::Sender<Result<T, DesktopError>>;

pub(crate) enum Payload {
    Json(Value),
    Binary(BinaryMsgType, Vec<u8>),
}

impl Payload {
    pub(crate) fn json(self, kind: Option<BinaryMsgType>) -> Result<Value, DesktopError> {
        match self {
            Self::Json(value) => Ok(value),
            Self::Binary(actual, bytes) if Some(actual) == kind => {
                Ok(serde_json::from_slice(&bytes)?)
            }
            Self::Binary(..) => Err(DesktopError::with_code(
                "protocol",
                "Unexpected binary message type",
            )),
        }
    }

    pub(crate) fn binary(self, kind: BinaryMsgType) -> Result<Vec<u8>, DesktopError> {
        match self {
            Self::Binary(actual, bytes) if actual == kind => Ok(bytes),
            _ => Err(DesktopError::with_code(
                "protocol",
                "Unexpected binary message type",
            )),
        }
    }
}

enum Command {
    Attach(String, Reply<SessionOverview>),
    Disconnect(Reply<()>),
    Offline(Box<Recording>, Option<Vec<u8>>, Reply<()>),
    Reconnect(Reply<()>),
    Channels(Vec<Channel>, Reply<()>),
    Pause(bool),
    TrackLayout(u64, bool),
    Clear(String),
    Rpc(u64, &'static str, Value, Reply<Payload>),
    Watch(TypedSink<TickSummary>, CancelToken),
}

pub(crate) struct Client {
    send: Option<mpsc::SyncSender<Command>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.send.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Client {
    pub(crate) fn spawn(store: Shared) -> io::Result<Self> {
        let (send, receive) = mpsc::sync_channel(64);
        let worker = thread::Builder::new()
            .name("devtools-agent".to_owned())
            .spawn(move || Worker::new(store, receive).run())?;
        Ok(Self {
            send: Some(send),
            worker: Some(worker),
        })
    }

    fn enqueue(&self, command: Command) -> Result<(), DesktopError> {
        self.send
            .as_ref()
            .ok_or_else(DesktopError::disconnected)?
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    DesktopError::with_code("busy", "DevTools command queue is full")
                }
                mpsc::TrySendError::Disconnected(_) => DesktopError::disconnected(),
            })
    }

    async fn request<T>(
        &self,
        command: impl FnOnce(Reply<T>) -> Command,
    ) -> Result<T, DesktopError> {
        let (send, receive) = oneshot::channel();
        self.enqueue(command(send))?;
        receive.await.map_err(|_| DesktopError::disconnected())?
    }

    pub(crate) async fn attach(&self, id: String) -> Result<SessionOverview, DesktopError> {
        self.request(|reply| Command::Attach(id, reply)).await
    }

    pub(crate) async fn disconnect(&self) -> Result<(), DesktopError> {
        self.request(Command::Disconnect).await
    }

    pub(crate) async fn offline(
        &self,
        data: Recording,
        trace: Option<Vec<u8>>,
    ) -> Result<(), DesktopError> {
        self.request(|reply| Command::Offline(Box::new(data), trace, reply))
            .await
    }

    pub(crate) async fn reconnect(&self) -> Result<(), DesktopError> {
        self.request(Command::Reconnect).await
    }

    pub(crate) async fn channels(&self, channels: Vec<Channel>) -> Result<(), DesktopError> {
        self.request(|reply| Command::Channels(channels, reply))
            .await
    }

    pub(crate) fn pause(&self, paused: bool) -> Result<(), DesktopError> {
        self.enqueue(Command::Pause(paused))
    }

    pub(crate) fn track_layout(&self, generation: u64, enabled: bool) -> Result<(), DesktopError> {
        self.enqueue(Command::TrackLayout(generation, enabled))
    }

    pub(crate) fn clear(&self, scope: String) -> Result<(), DesktopError> {
        self.enqueue(Command::Clear(scope))
    }

    pub(crate) async fn rpc(
        &self,
        generation: u64,
        method: &'static str,
        params: Value,
    ) -> Result<Payload, DesktopError> {
        self.request(|reply| Command::Rpc(generation, method, params, reply))
            .await
    }

    pub(crate) fn watch(
        &self,
        sink: TypedSink<TickSummary>,
        cancel: CancelToken,
    ) -> Result<(), DesktopError> {
        self.enqueue(Command::Watch(sink, cancel))
    }
}

struct Pending {
    deadline: Instant,
    action: Action,
}

enum Action {
    Reply(Reply<Payload>),
    Subscribe,
    Layout,
    SessionInfo,
}

struct Connection {
    socket: WebSocket<TcpStream>,
    _tunnel: Option<Tunnel>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    ping_sent: Option<Instant>,
    next_ping: Instant,
    subscriptions: Option<Vec<Channel>>,
}

impl Connection {
    fn open(target: &Target) -> Result<(Self, SessionInfo, Vec<String>), DesktopError> {
        let (port, token, tunnel) = target.connection().map_err(DesktopError::new)?;
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(250));
        if tunnel.is_some() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while stream.is_err() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(30));
                stream = TcpStream::connect_timeout(&address, Duration::from_millis(250));
            }
        }
        let stream = stream?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let (socket, _) = tungstenite::client::client_with_config(
            format!("ws://{address}/devtools"),
            stream,
            Some(
                tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(MAX_BINARY_PAYLOAD + BINARY_HEADER_LEN))
                    .max_frame_size(Some(MAX_BINARY_PAYLOAD + BINARY_HEADER_LEN)),
            ),
        )
        .map_err(|_| DesktopError::with_code("protocol", "DevTools WebSocket handshake failed"))?;
        let mut connection = Self {
            socket,
            _tunnel: tunnel,
            next_id: 0,
            pending: HashMap::new(),
            ping_sent: None,
            next_ping: Instant::now() + PING_INTERVAL,
            subscriptions: None,
        };
        let auth_id = connection.send(
            "authenticate",
            json!({"token": token, "version": PROTOCOL_VERSION_V2}),
        )?;
        let Message::Text(message) = connection.socket.read().map_err(ws_error)? else {
            return Err(DesktopError::with_code(
                "protocol",
                "Expected authentication reply",
            ));
        };
        let mut envelope: Value = serde_json::from_str(&message)?;
        if envelope["id"].as_u64() != Some(auth_id) {
            return Err(DesktopError::with_code(
                "protocol",
                "Invalid authentication reply",
            ));
        }
        let mut result = response(&mut envelope)?.json(None)?;
        if result["version"].as_u64() != Some(u64::from(PROTOCOL_VERSION_V2)) {
            return Err(DesktopError::with_code(
                "protocol",
                "Target does not support protocol v2",
            ));
        }
        let info = serde_json::from_value(result["session"].take())?;
        let capabilities = serde_json::from_value(result["capabilities"].take())?;
        connection
            .socket
            .get_mut()
            .set_read_timeout(Some(READ_TIMEOUT))?;
        Ok((connection, info, capabilities))
    }

    fn send(&mut self, method: &str, params: Value) -> Result<u64, DesktopError> {
        self.next_id += 1;
        let id = self.next_id;
        self.socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params,
                })
                .to_string()
                .into(),
            ))
            .map_err(ws_error)?;
        Ok(id)
    }

    fn send_pending(
        &mut self,
        method: &str,
        params: Value,
        action: Action,
    ) -> Result<(), DesktopError> {
        match self.send(method, params) {
            Ok(id) => {
                self.pending.insert(
                    id,
                    Pending {
                        deadline: Instant::now() + RPC_TIMEOUT,
                        action,
                    },
                );
                Ok(())
            }
            Err(error) => {
                if let Action::Reply(reply) = action {
                    let _ = reply.send(Err(DesktopError::with_code("io", &error)));
                }
                Err(error)
            }
        }
    }

    fn subscribe(
        &mut self,
        channels: Vec<Channel>,
        cursors: &ChannelCursors,
    ) -> Result<(), DesktopError> {
        self.send_pending(
            "subscribe",
            json!({"channels": channels, "cursors": cursors}),
            Action::Subscribe,
        )?;
        self.subscriptions = Some(channels);
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        for (_, pending) in self.pending.drain() {
            if let Action::Reply(reply) = pending.action {
                let _ = reply.send(Err(DesktopError::disconnected()));
            }
        }
    }
}

fn ws_error(error: tungstenite::Error) -> DesktopError {
    DesktopError::with_code("io", error)
}

fn response(value: &mut Value) -> Result<Payload, DesktopError> {
    match value.get_mut("result") {
        Some(result) => Ok(Payload::Json(result.take())),
        None => Err(DesktopError::with_code(
            "remote",
            value["error"]["message"]
                .as_str()
                .unwrap_or("Invalid DevTools RPC response"),
        )),
    }
}

fn binary(bytes: &[u8]) -> Result<(u64, Payload), DesktopError> {
    let (header, payload) = BinaryEnvelopeHeader::decode(bytes)
        .map_err(|error| DesktopError::with_code("protocol", error))?;
    let payload = if header.flags & BINARY_FLAG_ZSTD == 0 {
        payload.to_vec()
    } else {
        let mut decoded = Vec::new();
        zstd::stream::read::Decoder::new(payload)?
            .take((MAX_BINARY_PAYLOAD + 1) as u64)
            .read_to_end(&mut decoded)?;
        if decoded.len() > MAX_BINARY_PAYLOAD {
            return Err(DesktopError::with_code(
                "protocol",
                "Expanded binary response exceeds limit",
            ));
        }
        decoded
    };
    Ok((header.request_id, Payload::Binary(header.msg_type, payload)))
}

struct Watch {
    sink: TypedSink<TickSummary>,
    cancel: CancelToken,
    sent: Option<(u64, Option<FrameWindowKey>)>,
}

fn advance_tick(deadline: Instant, now: Instant) -> Instant {
    let late = now.duration_since(deadline);
    let phase = u64::try_from(late.as_nanos() % TICK_INTERVAL.as_nanos()).unwrap_or_default();
    now + (TICK_INTERVAL - Duration::from_nanos(phase))
}

struct Worker {
    store: Shared,
    commands: mpsc::Receiver<Command>,
    watches: Vec<Watch>,
    target: Option<Target>,
    connection: Option<Connection>,
    cursors: ChannelCursors,
    channels: Vec<Channel>,
    next_tick: Instant,
    next_discovery: Instant,
    next_audit: Instant,
    audited: u64,
    next_retry: Option<Instant>,
    next_layout: Option<Instant>,
    next_net_tick: Instant,
    failed_at: Option<Instant>,
    attempt: u32,
}

impl Worker {
    fn new(store: Shared, commands: mpsc::Receiver<Command>) -> Self {
        Self {
            store,
            commands,
            watches: Vec::new(),
            target: None,
            connection: None,
            cursors: ChannelCursors::default(),
            channels: DEFAULT_CHANNELS.to_vec(),
            next_tick: Instant::now(),
            next_discovery: Instant::now(),
            next_audit: Instant::now(),
            audited: u64::MAX,
            next_retry: None,
            next_layout: None,
            next_net_tick: Instant::now(),
            failed_at: None,
            attempt: 0,
        }
    }

    fn run(mut self) {
        loop {
            if self.connection.is_none() {
                match self.commands.recv_timeout(READ_TIMEOUT) {
                    Ok(command) => self.command(command),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            for _ in 0..16 {
                match self.commands.try_recv() {
                    Ok(command) => self.command(command),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            let now = Instant::now();
            if now >= self.next_discovery {
                self.next_discovery = now + DISCOVERY_INTERVAL;
                let mut targets: Vec<_> = discovery::targets()
                    .iter()
                    .map(Target::summary)
                    .take(128)
                    .collect();
                targets.sort_by_key(|target| target.is_self);
                let mut store = self
                    .store
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                if targets != store.targets {
                    store.targets = targets;
                    store.targets_rev = store.targets_rev.wrapping_add(1);
                    store.touch();
                }
            }
            if let Some(at) = self.next_retry
                && at <= now
            {
                self.retry();
            }
            if let Some(mut connection) = self.connection.take() {
                match self.drive(&mut connection, now) {
                    Ok(()) => self.connection = Some(connection),
                    Err(error) => self.lost(error),
                }
            }
            if now >= self.next_audit {
                self.next_audit = now + Duration::from_millis(500);
                let candidate = {
                    let state = self.store.read().unwrap_or_else(|error| error.into_inner());
                    (self.audited != state.sequence).then(|| {
                        (
                            state.sequence,
                            crate::analyzer::audits::evaluate(&state.data),
                        )
                    })
                };
                if let Some((sequence, findings)) = candidate {
                    let mut state = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    if state.sequence == sequence {
                        if state.findings != findings {
                            state.findings = findings;
                            state.audits_rev = state.audits_rev.wrapping_add(1);
                            state.touch();
                        }
                        self.audited = state.sequence;
                    }
                }
            }
            if now >= self.next_net_tick {
                self.next_net_tick = now + Duration::from_millis(250);
                let mut state = self
                    .store
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                if state.state == ConnectionState::Connected
                    && !state.paused
                    && state
                        .data
                        .network
                        .values()
                        .any(crate::store::network::active)
                {
                    state.refresh_network_ages();
                }
            }
            let now = Instant::now();
            if now >= self.next_tick {
                self.next_tick = advance_tick(self.next_tick, now);
                self.publish();
            }
        }
    }

    fn publish(&mut self) {
        self.watches.retain(|watch| !watch.cancel.is_cancelled());
        if self.watches.is_empty() {
            return;
        }
        let store = self.store.read().unwrap_or_else(|error| error.into_inner());
        let key = (store.sequence, store.frame_window_key());
        if self.watches.iter().all(|watch| watch.sent == Some(key)) {
            return;
        }
        let tick = store.tick();
        drop(store);
        for watch in &mut self.watches {
            if watch.sent != Some(key) && watch.sink.try_send(tick.clone()).is_ok() {
                watch.sent = Some(key);
            }
        }
    }

    fn connect(&mut self, candidate: Target) -> Result<SessionOverview, DesktopError> {
        let (connection, info, capabilities) = Connection::open(&candidate)?;
        let (project, framework) = candidate.roots();
        let target = candidate.summary();
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let new_process = store.data.info.as_ref().is_some_and(|previous| {
            previous.pid != info.pid
                || self
                    .target
                    .as_ref()
                    .is_some_and(|target| !target.same_registration(&candidate))
        });
        if new_process {
            self.cursors = ChannelCursors::default();
            store.replace(Recording::default());
        }
        if store.symbols.is_none() {
            drop(store);
            let symbols = candidate
                .symbol_path()
                .and_then(|path| SymbolTable::load(path).ok());
            store = self
                .store
                .write()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(symbols) = symbols {
                store.install_symbols(Arc::new(symbols));
            }
        }
        let overview = SessionOverview::from(&info);
        store.set_session_info(info);
        store.state = ConnectionState::Connected;
        store.set_capabilities(capabilities);
        if store.profile.is_empty() {
            store.profile = "counters".to_owned();
        }
        store.project_root = project;
        store.framework_root = framework;
        store.target_id = Some(target.id);
        store.inspecting_self = target.is_self;
        store.reconnected_new_pid = new_process;
        store.reconnect_attempt = 0;
        store.message = None;
        store.touch();
        self.connection = Some(connection);
        self.target = Some(candidate);
        self.failed_at = None;
        self.next_retry = None;
        self.attempt = 0;
        Ok(overview)
    }

    fn retry(&mut self) {
        self.next_retry = None;
        if self
            .failed_at
            .is_some_and(|time| time.elapsed() >= RETRY_WINDOW)
        {
            let mut store = self
                .store
                .write()
                .unwrap_or_else(|error| error.into_inner());
            store.state = ConnectionState::Offline;
            store.touch();
            return;
        }
        let Some(previous) = self.target.clone() else {
            return;
        };
        let mut candidates = discovery::targets();
        let candidate = candidates
            .iter()
            .position(|candidate| candidate.same_registration(&previous))
            .or_else(|| {
                candidates
                    .iter()
                    .position(|candidate| candidate.matches(&previous))
            })
            .map(|index| candidates.swap_remove(index));
        let result = self.connect(candidate.unwrap_or(previous));
        if let Err(error) = result {
            self.lost(error);
        }
    }

    fn lost(&mut self, error: DesktopError) {
        let now = Instant::now();
        self.failed_at.get_or_insert(now);
        self.next_layout = None;
        let backoff = 250 * (1_u64 << self.attempt.min(4));
        self.next_retry = Some(now + Duration::from_millis(backoff));
        self.attempt = self.attempt.saturating_add(1);
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        store.state = ConnectionState::Reconnecting;
        store.capture_id = None;
        store.capture_sampler = false;
        store.sampler_enabled = false;
        store.rtt_ms = None;
        store.reconnect_attempt = self.attempt;
        store.message = Some(error.to_string());
        store.touch();
    }

    fn clear_connection(&mut self, data: Recording, trace: Option<Vec<u8>>) {
        self.connection = None;
        self.target = None;
        self.next_retry = None;
        self.next_layout = None;
        self.failed_at = None;
        self.attempt = 0;
        self.cursors = ChannelCursors::default();
        let mut store = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        store.replace(data);
        if let Some(trace) = trace {
            store.trace = Some((store.data.capture["id"].as_u64().unwrap_or(0), trace.into()));
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Attach(id, reply) => {
                let Some(target) = discovery::targets()
                    .into_iter()
                    .find(|target| target.summary().id == id)
                else {
                    let _ = reply.send(Err(DesktopError::new("Target session is unavailable")));
                    return;
                };
                self.clear_connection(Recording::default(), None);
                {
                    let mut store = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    store.state = ConnectionState::Connecting;
                    store.target_id = Some(id);
                    store.touch();
                }
                self.publish();
                let result = self.connect(target);
                if let Err(error) = &result {
                    let mut store = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    store.state = ConnectionState::Idle;
                    store.message = Some(error.to_string());
                    store.touch();
                }
                let _ = reply.send(result);
            }
            Command::Disconnect(reply) => {
                self.clear_connection(Recording::default(), None);
                let _ = reply.send(Ok(()));
            }
            Command::Offline(data, trace, reply) => {
                self.clear_connection(*data, trace);
                let mut store = self
                    .store
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                store.state = ConnectionState::Offline;
                store.touch();
                let _ = reply.send(Ok(()));
            }
            Command::Reconnect(reply) => {
                if self.connection.is_none() && self.target.is_some() {
                    self.failed_at = None;
                    self.attempt = 0;
                    self.next_retry = Some(Instant::now());
                    let _ = reply.send(Ok(()));
                } else {
                    let _ = reply.send(Err(DesktopError::new("No previous target to reconnect")));
                }
            }
            Command::Channels(channels, reply) => {
                self.channels = channels;
                let _ = reply.send(Ok(()));
            }
            Command::Pause(paused) => {
                let mut store = self
                    .store
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                if store.paused != paused {
                    store.paused = paused;
                    store.touch();
                }
                if !paused && let Some(connection) = &mut self.connection {
                    connection.subscriptions = None;
                }
            }
            Command::TrackLayout(generation, enabled) => {
                if self
                    .store
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
                    .generation
                    == generation
                {
                    self.next_layout = enabled.then(Instant::now);
                }
            }
            Command::Clear(scope) => {
                let mut store = self
                    .store
                    .write()
                    .unwrap_or_else(|error| error.into_inner());
                // Keep server cursors so the next increment does not repopulate cleared history.
                match scope.as_str() {
                    "frames" => {
                        store.data.frames = Default::default();
                        store.clear_frame_clock();
                        store.changed(Channel::Frames);
                    }
                    "network" => {
                        store.data.network.clear();
                        store.data.ws.clear();
                        store.bodies = Default::default();
                        store.changed(Channel::Network);
                    }
                    "logs" => {
                        store.data.logs.clear();
                        store.data.errors.clear();
                        store.data.resolved_errors.clear();
                        store.changed(Channel::Logs);
                        store.changed(Channel::Errors);
                    }
                    "repaints" => {
                        store.data.repaints.clear();
                        store.changed(Channel::Repaints);
                    }
                    "all" => {
                        let mut data = Recording::default();
                        data.info = store.data.info.take();
                        store.data = data;
                        store.clear_frame_clock();
                        store.bodies = Default::default();
                        store.trace = None;
                        for (channel, _) in CHANNELS {
                            store.changed(channel);
                        }
                    }
                    _ => {}
                }
            }
            Command::Rpc(generation, method, params, reply) => {
                let result = self
                    .store
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
                    .generation
                    == generation;
                let Some(connection) = self.connection.as_mut().filter(|_| result) else {
                    let _ = reply.send(Err(DesktopError::disconnected()));
                    return;
                };
                if connection.pending.len() >= MAX_PENDING {
                    let _ = reply.send(Err(DesktopError::with_code(
                        "busy",
                        "Too many pending requests",
                    )));
                } else if let Err(error) =
                    connection.send_pending(method, params, Action::Reply(reply))
                {
                    self.connection = None;
                    self.lost(error);
                }
            }
            Command::Watch(sink, cancel) => {
                self.watches.retain(|watch| !watch.cancel.is_cancelled());
                if self.watches.len() < 8 {
                    self.watches.push(Watch {
                        sink,
                        cancel,
                        sent: None,
                    });
                }
            }
        }
    }

    fn drive(&mut self, connection: &mut Connection, now: Instant) -> Result<(), DesktopError> {
        if connection.subscriptions.as_ref() != Some(&self.channels) {
            connection.subscribe(self.channels.clone(), &self.cursors)?;
        }
        if self.next_layout.is_some_and(|next| now >= next) {
            self.next_layout = Some(now + Duration::from_millis(850));
            let paused = self
                .store
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .paused;
            if !paused
                && !connection
                    .pending
                    .values()
                    .any(|pending| matches!(pending.action, Action::Layout))
            {
                connection.send_pending("view.layout_issues", Value::Null, Action::Layout)?;
            }
        }
        if now >= connection.next_ping {
            if connection.ping_sent.is_some() {
                return Err(DesktopError::with_code(
                    "timeout",
                    "Target did not respond to ping",
                ));
            }
            connection
                .socket
                .send(Message::Ping(b"impasto".as_slice().into()))
                .map_err(ws_error)?;
            connection.send_pending("session.info", Value::Null, Action::SessionInfo)?;
            connection.ping_sent = Some(now);
            connection.next_ping = now + PING_INTERVAL;
        }
        let message = match connection.socket.read() {
            Ok(message) => Some(message),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(error) => return Err(ws_error(error)),
        };
        match message {
            Some(Message::Text(message)) => {
                let mut value: Value = serde_json::from_str(&message)?;
                if let Some(id) = value["id"].as_u64() {
                    self.complete(connection, id, response(&mut value))?;
                } else if value["method"] == "stream.batch" {
                    self.ingest(value["params"].take());
                }
            }
            Some(Message::Binary(bytes)) => {
                let (id, payload) = binary(&bytes)?;
                self.complete(connection, id, Ok(payload))?;
            }
            Some(Message::Pong(bytes)) if bytes.as_ref() == b"impasto" => {
                if let Some(sent) = connection.ping_sent.take() {
                    let mut store = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    let rtt = sent.elapsed().as_secs_f64() * 1000.;
                    if store.rtt_ms.is_none_or(|last| (last - rtt).abs() >= 0.5) {
                        store.rtt_ms = Some(rtt);
                        store.touch();
                    }
                }
            }
            Some(Message::Ping(_)) => connection.socket.flush().map_err(ws_error)?,
            Some(Message::Close(_)) => return Err(DesktopError::disconnected()),
            _ => {}
        }
        let expired: Vec<_> = connection
            .pending
            .iter()
            .filter_map(|(id, pending)| (now >= pending.deadline).then_some(*id))
            .collect();
        for id in expired {
            self.complete(
                connection,
                id,
                Err(DesktopError::with_code(
                    "timeout",
                    "DevTools request timed out",
                )),
            )?;
        }
        Ok(())
    }

    fn complete(
        &mut self,
        connection: &mut Connection,
        id: u64,
        payload: Result<Payload, DesktopError>,
    ) -> Result<(), DesktopError> {
        match connection.pending.remove(&id).map(|pending| pending.action) {
            Some(Action::Reply(reply)) => {
                let _ = reply.send(payload);
            }
            Some(Action::Subscribe) => self.ingest(payload?.json(None)?),
            Some(Action::SessionInfo) => {
                if let Ok(info) = payload
                    .and_then(|payload| payload.json(None))
                    .and_then(|value| {
                        serde_json::from_value::<SessionInfo>(value).map_err(Into::into)
                    })
                {
                    let mut state = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    state.set_session_info(info);
                }
            }
            Some(Action::Layout) => {
                if let Ok(layout) = payload.and_then(|payload| payload.json(None)) {
                    let mut state = self
                        .store
                        .write()
                        .unwrap_or_else(|error| error.into_inner());
                    if !state.paused && state.data.layouts != layout {
                        state.data.layouts = layout;
                        state.changed(Channel::Repaints);
                    }
                } else {
                    self.next_layout = None;
                }
            }
            None => {}
        }
        Ok(())
    }

    fn ingest(&mut self, batch: Value) {
        let batch = crate::store::decode_batch(batch);
        let mut resolved = std::collections::BTreeMap::new();
        let symbols = self
            .store
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .symbols
            .clone();
        if let Some(symbols) = &symbols {
            for incoming in &batch {
                if let crate::store::Incoming::Channel(Channel::Errors, update) = incoming
                    && !update.entries.is_empty()
                {
                    let resolver = symbols.resolver();
                    for entry in &update.entries {
                        resolved.insert(
                            crate::types::id(&entry["seq"]),
                            resolver.remap(crate::types::string(entry, "stack")),
                        );
                    }
                }
            }
        }
        let mut state = self
            .store
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if !symbols
            .as_ref()
            .zip(state.symbols.as_ref())
            .is_some_and(|(used, current)| Arc::ptr_eq(used, current))
        {
            resolved.clear();
        }
        state.ingest(batch, &mut self.cursors, resolved);
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use devtools_protocol::LocalSessionEntry;

    use super::*;

    #[test]
    fn overdue_ticks_keep_their_phase_and_skip_missed_slots() {
        let deadline = Instant::now();
        for elapsed in [0, 25, 99, 100, 211] {
            let now = deadline + Duration::from_millis(elapsed);
            let next = advance_tick(deadline, now);
            assert_eq!(
                next.duration_since(deadline),
                Duration::from_millis((elapsed / 100 + 1) * 100),
            );
            assert!(next > now);
        }
    }

    fn take_text(socket: &mut WebSocket<TcpStream>) -> Result<Value, DesktopError> {
        loop {
            match socket.read().map_err(ws_error)? {
                Message::Text(text) => return Ok(serde_json::from_str(&text)?),
                Message::Ping(_) => socket.flush().map_err(ws_error)?,
                _ => return Err(DesktopError::new("Unexpected mock WebSocket request")),
            }
        }
    }

    fn reply(
        socket: &mut WebSocket<TcpStream>,
        id: &Value,
        result: Value,
    ) -> Result<(), DesktopError> {
        socket
            .send(Message::Text(
                json!({"jsonrpc": "2.0", "id": id, "result": result})
                    .to_string()
                    .into(),
            ))
            .map_err(ws_error)
    }

    fn event(socket: &mut WebSocket<TcpStream>, batch: Value) -> Result<(), DesktopError> {
        socket
            .send(Message::Text(
                json!({"jsonrpc": "2.0", "method": "stream.batch", "params": batch})
                    .to_string()
                    .into(),
            ))
            .map_err(ws_error)
    }

    fn accept(listener: &TcpListener, pid: u32) -> Result<WebSocket<TcpStream>, DesktopError> {
        let (stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(RPC_TIMEOUT))?;
        let mut socket =
            tungstenite::accept(stream).map_err(|error| DesktopError::new(error.to_string()))?;
        let hello = take_text(&mut socket)?;
        assert_eq!(hello["method"], "authenticate");
        assert_eq!(hello["params"]["token"], "0123456789abcdef0123456789abcdef");
        reply(
            &mut socket,
            &hello["id"],
            json!({
                "version": 2, "capabilities": ["binary_frames"], "session": {
                    "pid": pid, "app_name": "Mock", "platform": "macos", "os": "macOS", "framework_version": "test",
                    "width": 400, "height": 300, "dpr": 2
                }
            }),
        )?;
        Ok(socket)
    }

    fn test_wait(check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !check() {
            assert!(Instant::now() < deadline, "Mock client condition timed out");
            thread::sleep(Duration::from_millis(8));
        }
    }

    #[test]
    fn duplex_batches_binary_correlation_and_new_process_reconnect() -> Result<(), DesktopError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let frame = |revision| devtools_protocol::FrameRecord {
            id: 7,
            revision,
            e2e_ms: 10.,
            ..Default::default()
        };
        let (resume, unblock) = mpsc::channel();
        let agent = thread::spawn(move || -> Result<(), DesktopError> {
            let mut socket = accept(&listener, 1)?;
            let request = take_text(&mut socket)?;
            assert_eq!(request["method"], "subscribe");
            assert_eq!(
                request["params"]["channels"].as_array().map(Vec::len),
                Some(7)
            );
            reply(
                &mut socket,
                &request["id"],
                json!({
                    "frames": {"reset": true, "revision": 1, "sampled_at_ms": 1000., "entries": [frame(1)]},
                    "network": {"reset": true, "revision": 1, "entries": [{"token": 12}]},
                    "repaints": {"reset": true, "revision": 1, "entries": [{"seq": 1,"view.picked": {"node_id":"35","final": true}}]},
                    "logs": {"reset": true, "revision": 1, "entries": [{"seq": 1,"message":"first log"}]},
                    "errors": {"reset": true, "revision": 1, "entries": [{"seq": 1,"message":"first error"}]},
                    "runtime": {"reset": true, "revision": 1, "entries": [{"value":{"detail":{"memory_used_size":8}}}]},
                    "resources": {"reset": true, "revision": 1, "entries": [{"value":{"kind":"async","detail":{"counts":{"timers":4}}}}]}
                }),
            )?;
            let request = take_text(&mut socket)?;
            assert_eq!(request["method"], "net.get_body");
            event(
                &mut socket,
                json!({"frames": {"reset": false, "revision": 2, "sampled_at_ms": 1010., "entries": [frame(2)]}}),
            )?;
            let header = BinaryEnvelopeHeader {
                msg_type: BinaryMsgType::NetBody,
                flags: 0,
                request_id: request["id"]
                    .as_u64()
                    .ok_or_else(|| DesktopError::new("Missing mock RPC id"))?,
                payload_len: 3,
            };
            let mut bytes = header.encode().to_vec();
            bytes.extend_from_slice(b"abc");
            socket
                .send(Message::Binary(bytes.into()))
                .map_err(ws_error)?;
            socket.close(None).map_err(ws_error)?;
            drop(socket);
            unblock
                .recv_timeout(RPC_TIMEOUT)
                .map_err(DesktopError::new)?;
            let mut socket = accept(&listener, 2)?;
            let request = take_text(&mut socket)?;
            assert!(request["params"]["cursors"]["frames"].is_null());
            reply(
                &mut socket,
                &request["id"],
                json!({"frames":{"reset":true,"revision":3,"sampled_at_ms":1000.,"entries":[frame(3)]}}),
            )?;
            let _ = take_text(&mut socket);
            Ok(())
        });
        let store = Shared::default();
        let (send, receive) = mpsc::sync_channel(8);
        let mut worker = Worker::new(Arc::clone(&store), receive);
        worker.channels = CHANNELS.iter().map(|(channel, _)| *channel).collect();
        worker.connect(Target::Local(LocalSessionEntry {
            pid: 1,
            app_name: "Mock".to_owned(),
            platform: "macos".to_owned(),
            port,
            token: "0123456789abcdef0123456789abcdef".to_owned(),
            protocol_version: 2,
            project_root: None,
            framework_root: None,
            source_map: None,
            started_at_ms: 0,
        }))?;
        let run = thread::spawn(move || worker.run());
        let state = || store.read().unwrap_or_else(|error| error.into_inner());
        test_wait(|| state().data.frames.get(7).is_some());
        {
            let state = state();
            assert_eq!(state.data.frames.entries().len(), 1);
            assert_eq!(
                state.picked.as_ref().map(|(_, node)| node.as_str()),
                Some("35")
            );
            assert_eq!(state.data.network.len(), 1);
            assert_eq!(state.data.logs.len(), 1);
            assert_eq!(state.data.errors.len(), 1);
            assert_eq!(state.data.memory["memory_used_size"], 8);
            assert_eq!(state.data.async_resources["counts"]["timers"], 4);
        }
        let (reply, mut response) = oneshot::channel();
        send.send(Command::Rpc(0, "net.get_body", json!({"token": 12}), reply))
            .map_err(|_| DesktopError::disconnected())?;
        let deadline = Instant::now() + RPC_TIMEOUT;
        let body = loop {
            if let Some(value) = response.try_recv().map_err(DesktopError::new)? {
                break value?;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(8));
        };
        assert_eq!(body.binary(BinaryMsgType::NetBody)?, b"abc");
        test_wait(|| state().state == ConnectionState::Reconnecting);
        assert_eq!(
            state().data.frames.get(7).map(|frame| frame.revision),
            Some(2)
        );
        resume.send(()).map_err(DesktopError::new)?;
        test_wait(|| {
            state()
                .data
                .frames
                .get(7)
                .is_some_and(|frame| frame.revision == 3)
        });
        assert_eq!(state().data.info.as_ref().map(|info| info.pid), Some(2));
        assert!(state().reconnected_new_pid && state().data.network.is_empty());
        drop(send);
        run.join()
            .map_err(|_| DesktopError::new("Client thread panicked"))?;
        agent
            .join()
            .map_err(|_| DesktopError::new("Agent thread panicked"))??;
        Ok(())
    }
}
