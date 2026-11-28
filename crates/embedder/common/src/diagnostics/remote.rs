//! Loopback-only, authenticated DevTools transport. UI-owned queries use the host inbox.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine as _;
use devtools_protocol::{
    BinaryEnvelopeHeader, BinaryMsgType, Capability, MAX_BINARY_PAYLOAD, PROTOCOL_VERSION_V1,
    PROTOCOL_VERSION_V2, SessionInfo,
};
use subtle::ConstantTimeEq;

use crate::command::{Command, DevtoolsQuery};
use crate::host::Poster;

use futures_util::future::{self, Either};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{interval, sleep, timeout};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async_with_config,
    tungstenite::{
        Error as SocketError, Message,
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
    },
};

use super::hub::{CaptureOptions, TraceCapture};
use super::remote_stream::Subscription;
use super::telemetry::TelemetryProfile;
use super::timeline::{FieldValue, TraceDetail};
use super::{TelemetryHub, discovery, remote_frame, remote_views};

const FRAME_SPAN_LIMIT: usize = 512;

const REQUEST_LIMIT: usize = 64 * 1024;
const CLIENT_LIMIT: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const PENDING_RPC_LIMIT: usize = 8;

pub(crate) struct Config {
    port: u16,
    token: Arc<str>,
}

impl Config {
    fn parse(port: &str, token: &str) -> Option<Self> {
        let port = port.parse().ok().filter(|port| *port != 0)?;
        if !(32..=128).contains(&token.len()) || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self {
            port,
            token: Arc::from(token),
        })
    }

    pub(crate) fn configured(_log_dir: Option<&Path>) -> Option<Self> {
        let mode = std::env::var("IMPASTO_DEVTOOLS");
        if mode.as_deref() == Ok("0") {
            return None;
        }
        #[cfg(target_os = "android")]
        if let Some(path) = _log_dir
            .and_then(Path::parent)
            .map(|cache| cache.join("impasto-devtools"))
            && let Ok(file) = std::fs::File::open(&path)
        {
            use std::io::Read;
            let mut config = String::new();
            let read = file.take(256).read_to_string(&mut config);
            let _ = std::fs::remove_file(path);
            read.ok()?;
            let (port, token) = config.trim_end().split_once('\n')?;
            return Self::parse(port, token);
        }
        match (
            std::env::var("IMPASTO_DEVTOOLS_PORT"),
            std::env::var("IMPASTO_DEVTOOLS_TOKEN"),
        ) {
            (Ok(port), Ok(token)) => Self::parse(&port, &token),
            (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent))
                if cfg!(debug_assertions) || mode.as_deref() == Ok("1") =>
            {
                #[cfg(target_os = "android")]
                return None; // The CLI allocates the remotely forwarded port.
                #[cfg(not(target_os = "android"))]
                return discovery::token().ok().map(|token| Self {
                    port: 0,
                    token: token.into(),
                });
            }
            _ => None,
        }
    }
}

pub(crate) struct Server {
    task: JoinHandle<()>,
    _registration: Option<discovery::Registration>,
}

impl Server {
    pub(crate) fn start(
        config: Config,
        hub: Arc<TelemetryHub>,
        poster: Poster,
    ) -> io::Result<Self> {
        let runtime = crate::rt::rt().map_err(io::Error::other)?.handle();
        let _enter = runtime.enter();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port);
        let socket = std::net::TcpListener::bind(address)?;
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(socket)?;
        let address = listener.local_addr()?;
        let register = cfg!(target_os = "macos")
            || std::env::var_os("IMPASTO_DEVTOOLS_DISCOVERY_DIR").is_some();
        let registration = if register {
            match discovery::Registration::publish(address.port(), &config.token) {
                Ok(registration) => Some(registration),
                Err(error) => {
                    tracing::warn!(target: "impasto::devtools", %error, "DevTools discovery unavailable");
                    None
                }
            }
        } else {
            None
        };
        tracing::info!(target: "impasto::devtools", %address, "DevTools listening");
        let state = Arc::new(Agent {
            token: config.token,
            hub,
            poster: Some(poster),
            name: discovery::app_name(),
            os: crate::host::host_os_version(),
        });
        Ok(Self {
            task: runtime.spawn(listen(listener, state)),
            _registration: registration,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Agent {
    token: Arc<str>,
    hub: Arc<TelemetryHub>,
    poster: Option<Poster>,
    name: String,
    os: String,
}

impl Agent {
    fn info(&self) -> SessionInfo {
        let metrics = self.hub.metrics();
        let frame = self.hub.latest_frame();
        let refresh_rate = *self
            .hub
            .refresh_rate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        #[expect(
            clippy::cast_possible_truncation,
            reason = "window refresh rates use the existing f32 display contract"
        )]
        let inferred_refresh_rate = frame
            .as_ref()
            .filter(|frame| frame.budget_ms > 0.0)
            .map(|frame| (1000.0 / frame.budget_ms) as f32);
        SessionInfo {
            pid: std::process::id(),
            app_name: self.name.clone(),
            platform: std::env::consts::OS.to_owned(),
            os: self.os.clone(),
            framework_version: impasto::VERSION.to_owned(),
            width: metrics.width,
            height: metrics.height,
            dpr: metrics.device_pixel_ratio,
            refresh_rate: refresh_rate.or(inferred_refresh_rate),
            topology: frame
                .and_then(|frame| frame.schedule.topology)
                .map(str::to_owned),
        }
    }
}

async fn listen(listener: TcpListener, agent: Arc<Agent>) {
    let mut clients = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        while clients.try_join_next().is_some() {}
        if clients.len() < CLIENT_LIMIT {
            clients.spawn(serve(stream, Arc::clone(&agent)));
        }
    }
}

#[expect(
    clippy::result_large_err,
    reason = "the WebSocket handshake callback fixes its response type"
)]
fn handshake(request: &Request, response: Response) -> Result<Response, ErrorResponse> {
    // Native clients omit Origin. The independent file:// client sends null.
    if request.uri().path() == "/devtools"
        && request.headers().get_all("origin").iter().count() <= 1
        && request
            .headers()
            .get("origin")
            .is_none_or(|origin| origin == "null")
    {
        Ok(response)
    } else {
        let mut denial = ErrorResponse::new(None);
        *denial.status_mut() = http::StatusCode::FORBIDDEN;
        Err(denial)
    }
}

type Socket = WebSocketStream<TcpStream>;

async fn send(socket: &mut Socket, message: Message) -> bool {
    timeout(IO_TIMEOUT, socket.send(message))
        .await
        .is_ok_and(|result| result.is_ok())
}

fn text(value: Value) -> Message {
    Message::Text(value.to_string().into())
}

fn binary(id: u64, kind: BinaryMsgType, payload: &[u8]) -> Message {
    let Ok(payload_len) = u32::try_from(payload.len()) else {
        return text(error(Some(id), -32006, "payload too large"));
    };
    if payload.len() > MAX_BINARY_PAYLOAD {
        return text(error(Some(id), -32006, "payload too large"));
    }
    let header = BinaryEnvelopeHeader {
        msg_type: kind,
        flags: 0,
        request_id: id,
        payload_len,
    };
    let mut bytes = Vec::with_capacity(header.encode().len() + payload.len());
    bytes.extend(header.encode());
    bytes.extend_from_slice(payload);
    Message::Binary(bytes.into())
}

fn response(id: Option<u64>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Option<u64>, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn matches_token(expected: &str, given: &str) -> bool {
    expected.len() == given.len() && bool::from(expected.as_bytes().ct_eq(given.as_bytes()))
}

async fn serve(stream: TcpStream, agent: Arc<Agent>) {
    let hub = &agent.hub;
    let limits = WebSocketConfig::default()
        .max_message_size(Some(REQUEST_LIMIT))
        .max_frame_size(Some(REQUEST_LIMIT));
    let Ok(Ok(mut socket)) = timeout(
        IO_TIMEOUT,
        accept_hdr_async_with_config(stream, handshake, Some(limits)),
    )
    .await
    else {
        return;
    };
    let Ok(Some(Ok(Message::Text(message)))) = timeout(IO_TIMEOUT, socket.next()).await else {
        return;
    };
    let auth: Value = serde_json::from_str(&message).unwrap_or(Value::Null);
    let id = auth["id"].as_u64();
    let version = auth["params"]["version"]
        .as_u64()
        .and_then(|value| u8::try_from(value).ok());
    if auth["jsonrpc"] != "2.0"
        || id.is_none()
        || auth["method"] != "authenticate"
        || !matches!(version, Some(PROTOCOL_VERSION_V1 | PROTOCOL_VERSION_V2))
        || !auth["params"]["token"]
            .as_str()
            .is_some_and(|given| matches_token(&agent.token, given))
    {
        send(
            &mut socket,
            text(error(id, -32001, "authentication failed")),
        )
        .await;
        return;
    }
    let v2 = version == Some(PROTOCOL_VERSION_V2);
    let mut capabilities = vec![
        Capability::FrameSpans,
        Capability::CaptureStatus,
        Capability::TraceDetail,
        Capability::CaptureGpu,
        Capability::Inspector,
    ];
    let result = if v2 {
        capabilities.extend([
            Capability::MultiChannelSubscribe,
            Capability::BinaryFrames,
            Capability::ViewRepaintTracker,
            Capability::RemoteOverlay,
        ]);
        json!({"version": PROTOCOL_VERSION_V2, "capabilities": capabilities, "session": agent.info()})
    } else {
        json!({"version": PROTOCOL_VERSION_V1, "capabilities": capabilities})
    };
    if !send(&mut socket, text(response(id, result))).await {
        return;
    }
    let _inspector = (!v2).then(|| hub.inspect_remote());
    let mut cursor = None;
    let mut subscription = Subscription::new(Arc::clone(hub));
    let mut overlay = OverlayLease::default();
    let mut recording = None;
    let mut captures = JoinSet::new();
    let mut updates = interval(if v2 {
        UPDATE_INTERVAL
    } else {
        Duration::from_millis(250)
    });
    updates.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    enum Next {
        Incoming(Option<Result<Message, SocketError>>),
        Finished(Option<Message>),
        Update,
    }
    loop {
        let next = {
            let incoming = pin!(socket.next());
            let capture = pin!(async {
                if captures.is_empty() {
                    future::pending().await
                } else {
                    captures.join_next().await.and_then(Result::ok)
                }
            });
            let actions = pin!(future::select(incoming, capture));
            match future::select(pin!(updates.tick()), actions).await {
                Either::Left(_) => Next::Update,
                Either::Right((Either::Left((message, _)), _)) => Next::Incoming(message),
                Either::Right((Either::Right((result, _)), _)) => Next::Finished(result),
            }
        };
        let result = match next {
            Next::Incoming(Some(Ok(Message::Text(message)))) => {
                if v2 {
                    request_v2(
                        &agent,
                        &message,
                        &mut subscription,
                        &mut recording,
                        &mut overlay,
                        &mut captures,
                    )
                } else {
                    request(
                        hub,
                        agent.poster.as_ref(),
                        &message,
                        &mut cursor,
                        &mut captures,
                    )
                    .map(text)
                }
            }
            Next::Incoming(Some(Ok(Message::Ping(data)))) => {
                if !timeout(IO_TIMEOUT, socket.send(Message::Pong(data)))
                    .await
                    .is_ok_and(|result| result.is_ok())
                {
                    break;
                }
                continue;
            }
            Next::Incoming(Some(Ok(Message::Pong(_)))) => continue,
            Next::Incoming(_) => break,
            Next::Finished(result) => result,
            Next::Update if v2 => subscription.update(hub).map(text),
            Next::Update => cursor.and_then(|since| {
                let (revision, reset, samples) = hub.frame_updates(since);
                if revision == since {
                    return None;
                }
                cursor = Some(revision);
                Some(text(json!({
                    "jsonrpc": "2.0", "method": "frames",
                    "params": frame_update(revision, reset, &samples),
                })))
            }),
        };
        if let Some(result) = result
            && !send(&mut socket, result).await
        {
            break;
        }
    }
}

fn request(
    hub: &Arc<TelemetryHub>,
    poster: Option<&Poster>,
    message: &str,
    cursor: &mut Option<u64>,
    captures: &mut JoinSet<Message>,
) -> Option<Value> {
    let Ok(rpc) = serde_json::from_str::<Value>(message) else {
        return Some(error(None, -32700, "invalid JSON"));
    };
    let id = rpc["id"].as_u64();
    if rpc["jsonrpc"] != "2.0" || id.is_none() || !rpc["method"].is_string() {
        return Some(error(id, -32600, "invalid request"));
    }
    let params = &rpc["params"];
    Some(match rpc["method"].as_str() {
        Some("inspector") => {
            let cached = hub
                .inspector()
                .and_then(|snapshot| serde_json::from_str(&snapshot).ok());
            if cached.is_none()
                && let (Some(id), Some(poster)) = (id, poster)
                && captures.len() < PENDING_RPC_LIMIT
            {
                let (reply, done) = tokio::sync::oneshot::channel();
                if poster.post_devtools(Command::Devtools {
                    query: DevtoolsQuery::ViewSnapshot { max_nodes: 512 },
                    reply,
                }) {
                    captures.spawn(ui_reply(id, None, done));
                    return None;
                }
            }
            response(id, cached.unwrap_or(Value::Null))
        }
        Some("subscribe") => {
            let since = params["since"].as_u64().unwrap_or_default();
            let (revision, reset, samples) = hub.frame_updates(since);
            *cursor = Some(revision);
            response(id, frame_update(revision, reset, &samples))
        }
        Some("capture_status") => {
            let last = hub.latest_capture();
            response(
                id,
                json!({
                    "active": hub.capture_status().map(|(id, recording)| json!({
                        "id": id, "state": if recording { "recording" } else { "stopping" },
                    })),
                    "latest": last.as_ref().map(|archive| capture_summary(archive)),
                }),
            )
        }
        Some("get_frame_spans") => {
            match (params["capture_id"].as_u64(), params["frame_id"].as_u64()) {
                (Some(capture_id), Some(frame_id)) => {
                    if let Some(capture) = hub
                        .latest_capture()
                        .filter(|capture| capture.id == capture_id)
                        && let Some(details) = frame_spans(&capture, frame_id)
                    {
                        response(id, details)
                    } else {
                        error(id, -32003, "frame is outside the retained capture")
                    }
                }
                _ => error(id, -32602, "expected capture_id and frame_id"),
            }
        }
        Some("capture") => {
            let duration = params["duration_ms"].as_u64().unwrap_or(5000);
            if !(100..=5000).contains(&duration) {
                return Some(error(
                    id,
                    -32602,
                    "capture must run for 100 to 5000 milliseconds",
                ));
            }
            let detail = match params["detail"].as_str() {
                Some("detailed") => TraceDetail::Detailed,
                Some("stages") | None => TraceDetail::Stages,
                _ => return Some(error(id, -32602, "detail must be stages or detailed")),
            };
            if !captures.is_empty() {
                return Some(error(id, -32002, "trace capture busy"));
            }
            let gpu = match params["gpu"] {
                Value::Bool(gpu) => gpu,
                Value::Null => true,
                _ => return Some(error(id, -32602, "gpu must be boolean")),
            };
            let Some(capture_id) = hub.begin_capture(None, CaptureOptions { detail, gpu }) else {
                return Some(error(id, -32002, "trace capture busy"));
            };
            let capture = Capture {
                hub: Arc::clone(hub),
                id: capture_id,
            };
            captures.spawn(async move {
                match capture.finish(Duration::from_millis(duration)).await {
                    Some(trace) => text(response(id, trace)),
                    None => text(error(id, -32002, "trace capture unavailable")),
                }
            });
            return None;
        }
        _ => error(id, -32601, "unknown method"),
    })
}

fn request_v2(
    agent: &Agent,
    message: &str,
    subscription: &mut Subscription,
    recording: &mut Option<Capture>,
    overlay: &mut OverlayLease,
    pending: &mut JoinSet<Message>,
) -> Option<Message> {
    let Ok(rpc) = serde_json::from_str::<Value>(message) else {
        return Some(text(error(None, -32700, "invalid JSON")));
    };
    let (Some(id), Some(method), true) = (
        rpc["id"].as_u64(),
        rpc["method"].as_str(),
        rpc["jsonrpc"] == "2.0",
    ) else {
        return Some(text(error(rpc["id"].as_u64(), -32600, "invalid request")));
    };
    let ok = |result| text(response(Some(id), result));
    let fail = |code, message| text(error(Some(id), code, message));
    let params = &rpc["params"];
    let hub = &agent.hub;
    Some(match method {
        "session.info" => ok(json!(agent.info())),
        "subscribe" => match serde_json::from_value(params.clone()) {
            Ok(request) => ok(subscription.subscribe(request, hub)),
            Err(_) => fail(-32602, "expected channels and optional cursors"),
        },
        "telemetry.set_profile" => {
            let profile = match params["profile"].as_str() {
                Some("off") => TelemetryProfile::Off,
                Some("counters") => TelemetryProfile::Counters,
                Some("gpu_timeline") => TelemetryProfile::GpuAndTimeline,
                Some("deep_inspect") => TelemetryProfile::DeepInspect,
                _ => return Some(fail(-32602, "invalid telemetry profile")),
            };
            if let Some(tracking) = params.get("repaint_tracking") {
                let Some(enabled) = tracking.as_bool() else {
                    return Some(fail(-32602, "repaint_tracking must be boolean"));
                };
                subscription.set_repaint_tracking(enabled);
            }
            hub.set_profile(profile);
            ok(json!(true))
        }
        "frame.detail" => match params["frame_id"].as_u64() {
            Some(frame_id) => hub.frame_detail(frame_id).map_or_else(
                || fail(-32003, "frame is outside the retained window"),
                |frame| ok(json!(remote_frame::record(&frame))),
            ),
            None => fail(-32602, "expected frame_id"),
        },
        "frame.repaints" => {
            let Some(frame_id) = params["frame_id"].as_u64() else {
                return Some(fail(-32602, "expected frame_id"));
            };
            let Some(batch) = hub.frame_repaints(frame_id) else {
                return Some(fail(-32003, "no retained repaints for frame"));
            };
            let value = remote_views::repaints(&batch);
            if batch.entries.len() >= 64 {
                binary(
                    id,
                    BinaryMsgType::FrameRepaints,
                    value.to_string().as_bytes(),
                )
            } else {
                ok(value)
            }
        }
        "frame.layer_tree" => {
            let Some((frame, tree)) = hub.retained_layers(params["frame_id"].as_u64()) else {
                return Some(fail(-32003, "no retained layer tree"));
            };
            if pending.len() >= PENDING_RPC_LIMIT {
                return Some(fail(-32002, "too many pending requests"));
            }
            pending.spawn_blocking(move || {
                binary(
                    id,
                    BinaryMsgType::ViewSnapshot,
                    remote_views::layer_tree(frame, &tree)
                        .to_string()
                        .as_bytes(),
                )
            });
            return None;
        }
        "view.snapshot" | "view.probe" | "view.set_overlay" | "view.repaint_leaderboard" => {
            if pending.len() >= PENDING_RPC_LIMIT {
                return Some(fail(-32002, "too many pending requests"));
            }
            let Some(poster) = &agent.poster else {
                return Some(fail(-32004, "session closed"));
            };
            let query = match method {
                "view.snapshot" => DevtoolsQuery::ViewSnapshot {
                    max_nodes: usize::try_from(
                        params["max_nodes"].as_u64().unwrap_or(512).min(20_000),
                    )
                    .unwrap_or(512),
                },
                "view.probe" => {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "hit testing uses scene coordinates represented as f32"
                    )]
                    let coordinates = params["x"]
                        .as_f64()
                        .zip(params["y"].as_f64())
                        .map(|(x, y)| (x as f32, y as f32));
                    let Some((x, y)) = coordinates.filter(|(x, y)| x.is_finite() && y.is_finite())
                    else {
                        return Some(fail(-32602, "expected finite scene coordinates"));
                    };
                    DevtoolsQuery::ViewProbe { x, y }
                }
                "view.repaint_leaderboard" => DevtoolsQuery::ViewRepaintLeaderboard {
                    limit: usize::try_from(params["limit"].as_u64().unwrap_or(32).min(256))
                        .unwrap_or(32),
                },
                _ => {
                    let mut params = params.clone();
                    if let Some(node) = params.get_mut("highlight_node")
                        && let Some(raw) = node.as_u64()
                    {
                        *node = json!(raw.to_string());
                    }
                    let Ok(config) = serde_json::from_value(params) else {
                        return Some(fail(-32602, "invalid overlay configuration"));
                    };
                    DevtoolsQuery::ViewSetOverlay {
                        owner: overlay.claim(poster),
                        config,
                    }
                }
            };
            let (reply, done) = tokio::sync::oneshot::channel();
            if !poster.post_devtools(Command::Devtools { query, reply }) {
                return Some(fail(-32002, "UI queue is unavailable or busy"));
            }
            let binary_kind = (method == "view.snapshot").then_some(BinaryMsgType::ViewSnapshot);
            pending.spawn(ui_reply(id, binary_kind, done));
            return None;
        }
        "trace.start" => {
            let Some(options) = capture_options(params) else {
                return Some(fail(-32602, "invalid trace options"));
            };
            let Some(capture_id) = hub.begin_capture(None, options) else {
                return Some(fail(-32002, "trace capture busy"));
            };
            *recording = Some(Capture {
                hub: Arc::clone(hub),
                id: capture_id,
            });
            ok(json!({"id": capture_id}))
        }
        "trace.stop" => {
            if pending.len() >= PENDING_RPC_LIMIT {
                return Some(fail(-32002, "too many pending requests"));
            }
            let Some(capture) = recording.take() else {
                return Some(fail(-32003, "no active trace capture"));
            };
            pending.spawn_blocking(move || match capture.hub.take_capture(capture.id) {
                Some(archive) => text(response(Some(id), capture_summary(&archive))),
                None => text(error(Some(id), -32003, "trace capture unavailable")),
            });
            return None;
        }
        "trace.export_pftrace" => {
            let Some(archive) = hub.latest_capture().filter(|archive| {
                params["capture_id"].is_null() || params["capture_id"].as_u64() == Some(archive.id)
            }) else {
                return Some(fail(-32003, "trace capture unavailable"));
            };
            if pending.len() >= PENDING_RPC_LIMIT {
                return Some(fail(-32002, "too many pending requests"));
            }
            pending.spawn_blocking(move || {
                binary(
                    id,
                    BinaryMsgType::TraceExport,
                    &super::trace_export::encode(&archive),
                )
            });
            return None;
        }
        "trace.get_frame_spans" => match (
            hub.latest_capture()
                .filter(|archive| params["capture_id"].as_u64() == Some(archive.id)),
            params["frame_id"].as_u64(),
        ) {
            (Some(archive), Some(frame_id)) => frame_spans(&archive, frame_id)
                .map_or_else(|| fail(-32003, "frame is outside the retained capture"), ok),
            _ => fail(-32003, "capture or frame unavailable"),
        },
        _ => fail(-32601, "unknown method"),
    })
}

#[derive(Default)]
struct OverlayLease(Option<(u64, Poster)>);

impl OverlayLease {
    fn claim(&mut self, poster: &Poster) -> u64 {
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        self.0
            .get_or_insert_with(|| (NEXT_OWNER.fetch_add(1, Ordering::Relaxed), poster.clone()))
            .0
    }
}

impl Drop for OverlayLease {
    fn drop(&mut self) {
        if let Some((owner, poster)) = &self.0 {
            let (reply, abandoned) = tokio::sync::oneshot::channel();
            drop(abandoned);
            poster.post(Command::Devtools {
                query: DevtoolsQuery::ClearOverlay { owner: *owner },
                reply,
            });
        }
    }
}

fn capture_options(params: &Value) -> Option<CaptureOptions> {
    let detail = match params["detail"].as_str() {
        Some("detailed") => TraceDetail::Detailed,
        Some("stages") => TraceDetail::Stages,
        None if params["detail"].is_null() => TraceDetail::Stages,
        _ => return None,
    };
    let gpu = match params["gpu"] {
        Value::Bool(value) => value,
        Value::Null => true,
        _ => return None,
    };
    Some(CaptureOptions { detail, gpu })
}

async fn ui_reply(
    id: u64,
    kind: Option<BinaryMsgType>,
    done: tokio::sync::oneshot::Receiver<Result<Vec<u8>, String>>,
) -> Message {
    let value = match timeout(IO_TIMEOUT, done).await {
        Ok(Ok(Ok(payload))) => {
            if let Some(kind) = kind {
                return binary(id, kind, &payload);
            }
            serde_json::from_slice(&payload)
                .map(|value| response(Some(id), value))
                .unwrap_or_else(|_| error(Some(id), -32603, "invalid UI reply"))
        }
        Ok(Ok(Err(message))) => error(Some(id), -32004, &message),
        Ok(Err(_)) => error(Some(id), -32004, "session closed"),
        Err(_) => error(Some(id), -32005, "UI query timed out"),
    };
    text(value)
}

fn capture_summary(capture: &TraceCapture) -> Value {
    json!({
        "id": capture.id,
        "frame_ids": capture.frames.iter().map(|frame| frame.number).collect::<Vec<_>>(),
        "frames": capture.frames.iter().map(|frame| {
            frame_value(frame, FrameFields::Summary)
        }).collect::<Vec<_>>(),
        "overwritten_frames": capture.overwritten_frames,
        "unfinished_frames": capture.unfinished_frames,
        "overwritten_spans": capture.span_loss.overwritten,
        "open_spans_at_stop": capture.span_loss.open_at_stop,
        "producer_limit": capture.span_loss.producer_limit,
        "pending_gpu": capture.pending_gpu(),
    })
}

fn frame_spans(capture: &TraceCapture, frame_id: u64) -> Option<Value> {
    let frame = capture
        .frames
        .iter()
        .find(|frame| frame.number == frame_id)?;
    let mut candidates: Vec<_> = capture
        .spans
        .iter()
        .filter(|span| span.frame_id == Some(frame_id))
        .collect();
    candidates.sort_by_key(|span| (span.start_ns, span.id));
    let omitted = candidates.len().saturating_sub(FRAME_SPAN_LIMIT);
    let spans: Vec<_> = candidates.into_iter().take(FRAME_SPAN_LIMIT).map(|span| {
        let fields: serde_json::Map<String, Value> = span.fields.iter().map(|(key, value)| {
            let value = match value {
                FieldValue::I64(value) => json!(value),
                FieldValue::U64(value) => json!(value),
                FieldValue::F64(value) => json!(value),
                FieldValue::Bool(value) => json!(value),
                FieldValue::Str(value) => json!(value),
            };
            ((*key).to_owned(), value)
        }).collect();
        json!({
            "id": span.id, "parent_id": span.parent_id, "name": span.name,
            "queue": span.queue.name(), "thread": span.thread_name.as_ref(), "target": span.target,
            "offset_ns": span.start_ns.saturating_sub(frame.schedule.ui_start_ns),
            "duration_ns": span.end_ns.saturating_sub(span.start_ns),
            "instant": span.is_instant, "fields": fields,
        })
    }).collect();
    Some(json!({
        "capture_id": capture.id, "frame": frame_value(frame, FrameFields::Full),
        "spans": spans, "omitted_spans": omitted,
    }))
}

struct Capture {
    hub: Arc<TelemetryHub>,
    id: u64,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.hub.cancel_capture(self.id);
    }
}

impl Capture {
    async fn finish(self, duration: Duration) -> Option<Value> {
        sleep(duration).await;
        tokio::task::spawn_blocking(move || {
            let capture = self.hub.take_capture(self.id)?;
            let bytes = super::trace_export::encode(&capture);
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let summary = capture_summary(&capture);
            Some(json!({
                "format": "pftrace", "encoding": "base64", "bytes": bytes.len(),
                "data": encoded, "capture": summary,
            }))
        })
        .await
        .ok()?
    }
}

fn frame_update(revision: u64, reset: bool, samples: &[super::FrameSample]) -> Value {
    let frames: Vec<_> = samples
        .iter()
        .map(|frame| frame_value(frame, FrameFields::Full))
        .collect();
    json!({ "cursor": revision, "reset": reset, "frames": frames })
}

#[derive(Clone, Copy)]
enum FrameFields {
    Summary,
    Full,
}

fn frame_value(frame: &super::FrameSample, fields: FrameFields) -> Value {
    let render = frame.raster.render;
    let passes = frame
        .gpu_timing
        .as_ref()
        .filter(|_| matches!(fields, FrameFields::Full))
        .map(|gpu| {
            gpu.passes.iter().map(|pass| json!({
            "kind": format!("{:?}", pass.kind), "slice": pass.slice_index,
            "draws": pass.draws, "start_us": pass.start_us, "duration_us": pass.duration_us,
            "shader_id": pass.shader_id().map(|id| id.0), "view_id": pass.view_id(),
        })).collect::<Vec<_>>()
        });
    json!({
        "id": frame.number, "revision": frame.changed_at, "budget_ms": frame.budget_ms,
        "ui_ms": frame.schedule.ui_ms(), "raster_ms": frame.schedule.raster_ms(),
        "queue_ms": frame.schedule.queue_ms(), "e2e_ms": frame.schedule.e2e_ms(),
        "topology": frame.schedule.topology,
        "compositor_only": frame.schedule.compositor_only,
        "compositor_block": frame.schedule.compositor_block.map(|reason| json!({
            "reason": reason.name(), "node": reason.node().map(|id| id.to_string()),
        })),
        "over_budget": frame.over_budget, "draws": render.draws, "passes": render.passes,
        "gpu": {
            "state": frame.gpu.status.name(), "generation": frame.gpu.device_generation,
            "frame_span_us": frame.gpu_timing.as_ref().map(|gpu| gpu.frame_span_us),
            "passes_sum_us": frame.gpu_timing.as_ref().map(|gpu| gpu.passes_sum_us), "passes": passes,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raster::{GpuFrameTiming, GpuSample, GpuTimingStatus, GpuTimingUpdate};
    use tokio_tungstenite::{MaybeTlsStream, connect_async};

    type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn test_agent(token: Arc<str>, hub: &Arc<TelemetryHub>) -> Arc<Agent> {
        Arc::new(Agent {
            token,
            hub: Arc::clone(hub),
            poster: None,
            name: "Example".to_owned(),
            os: String::new(),
        })
    }

    async fn read(socket: &mut Client) -> Value {
        let message = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }

    async fn call(socket: &mut Client, id: u64, method: &str, params: Value) -> Value {
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": id, "method": method, "params": params
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        loop {
            let value = read(socket).await;
            if value["id"] == id {
                return value;
            }
        }
    }

    #[test]
    fn v2_batches_channels_with_independent_resets_and_returns_binary_replies() {
        crate::rt::rt().unwrap().handle().block_on(async {
            let hub = Arc::new(TelemetryHub::new());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/devtools", listener.local_addr().unwrap());
            let _server = Server {
                task: tokio::spawn(listen(listener, test_agent(TOKEN.into(), &hub))),
                _registration: None,
            };
            let (mut client, _) = connect_async(&url).await.unwrap();
            let auth = call(
                &mut client,
                1,
                "authenticate",
                json!({"version": 2, "token": TOKEN}),
            )
            .await;
            assert_eq!(auth["result"]["session"]["app_name"], "Example");
            let initial = call(
                &mut client,
                2,
                "subscribe",
                json!({
                    "channels": ["frames", "errors"],
                    "cursors": {"frames": 0, "errors": 123}
                }),
            )
            .await;
            assert_eq!(initial["result"]["frames"]["reset"], false);
            assert_eq!(initial["result"]["errors"]["reset"], true);
            hub.push_frame(super::super::FrameSample {
                number: 42,
                ..Default::default()
            });
            hub.error("test", "failure", None, Some(42));
            let batch = read(&mut client).await;
            assert_eq!(batch["method"], "stream.batch");
            assert_eq!(batch["params"]["frames"]["entries"][0]["id"], 42);
            assert_eq!(batch["params"]["errors"]["entries"][0]["frame"], 42);

            let (tx, rx) = tokio::sync::oneshot::channel();
            tx.send(Ok(b"[]".to_vec())).unwrap();
            let reply = ui_reply(17, Some(BinaryMsgType::ViewSnapshot), rx).await;
            let bytes = reply.into_data();
            let (header, payload) = BinaryEnvelopeHeader::decode(&bytes).unwrap();
            assert_eq!(header.request_id, 17);
            assert_eq!(payload, b"[]");
            let (tx, rx) = tokio::sync::oneshot::channel();
            drop(tx);
            let reply = ui_reply(18, None, rx).await;
            let reply: Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
            assert_eq!(reply["error"]["code"], -32004);
        });
    }

    #[test]
    fn subscription_backfills_gpu_and_exports_a_trace() {
        let _capture = super::super::hub::CAPTURE_TEST_LOCK.lock().unwrap();
        crate::rt::rt().unwrap().handle().block_on(async {
            let hub = Arc::new(TelemetryHub::new());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/devtools", listener.local_addr().unwrap());
            let config = Config::parse("8765", TOKEN).unwrap();
            let server = Server {
                task: tokio::spawn(listen(listener, test_agent(config.token, &hub))),
                _registration: None,
            };
            let (mut incompatible, _) = connect_async(&url).await.unwrap();
            let denied = call(
                &mut incompatible,
                1,
                "authenticate",
                json!({
                    "token": TOKEN, "version": 99
                }),
            )
            .await;
            assert_eq!(denied["error"]["code"], -32001);
            drop(incompatible);

            let (mut client, _) = connect_async(&url).await.unwrap();
            let auth = call(
                &mut client,
                2,
                "authenticate",
                json!({
                    "token": TOKEN, "version": 1
                }),
            )
            .await;
            assert_eq!(auth["result"]["version"], 1);
            hub.push_frame(super::super::FrameSample {
                number: 42,
                gpu: GpuSample {
                    frame_id: 42,
                    device_generation: 7,
                    status: GpuTimingStatus::Pending,
                },
                ..Default::default()
            });
            let reply = call(&mut client, 3, "subscribe", json!({})).await;
            assert_eq!(reply["result"]["frames"][0]["gpu"]["state"], "pending");
            let revision = reply["result"]["cursor"].as_u64().unwrap();
            hub.patch_gpu_timing(GpuTimingUpdate {
                sample: GpuSample {
                    frame_id: 42,
                    device_generation: 7,
                    status: GpuTimingStatus::Ready,
                },
                timing: Some(GpuFrameTiming {
                    clock_start_us: 3.0,
                    frame_span_us: 20.0,
                    passes_sum_us: 18.0,
                    passes: Vec::new(),
                }),
            });
            let update = read(&mut client).await;
            assert_eq!(update["method"], "frames");
            assert_eq!(update["params"]["cursor"], revision + 1);
            assert_eq!(update["params"]["frames"][0]["id"], 42);
            assert_eq!(update["params"]["frames"][0]["gpu"]["frame_span_us"], 20.0);

            client.send(Message::Text(json!({
                "jsonrpc": "2.0", "id": 4, "method": "capture", "params": { "duration_ms": 500 }
            }).to_string().into())).await.unwrap();
            let status = call(&mut client, 5, "capture_status", json!({})).await;
            assert_eq!(status["result"]["active"]["state"], "recording");
            hub.push_frame(super::super::FrameSample {
                number: 43,
                schedule: super::super::telemetry::ScheduleTiming {
                    ui_start_ns: super::super::telemetry::mono_ns(),
                    ..Default::default()
                },
                ..Default::default()
            });
            let update = read(&mut client).await;
            assert_eq!(update["method"], "frames");
            assert_eq!(update["params"]["frames"][0]["id"], 43);
            let reply = read(&mut client).await;
            assert_eq!(reply["id"], 4);
            let trace = &reply["result"];
            assert_eq!(trace["format"], "pftrace");
            assert_eq!(trace["capture"]["frame_ids"], json!([43]));
            let detail = call(
                &mut client,
                6,
                "get_frame_spans",
                json!({
                    "capture_id": trace["capture"]["id"], "frame_id": 43,
                }),
            )
            .await;
            assert_eq!(detail["result"]["frame"]["id"], 43);
            assert_eq!(reply["result"]["encoding"], "base64");
            let encoded = reply["result"]["data"].as_str().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            assert_eq!(
                reply["result"]["bytes"].as_u64().unwrap(),
                bytes.len() as u64
            );
            let decoded =
                <super::super::pftrace::Trace as prost::Message>::decode(&bytes[..]).unwrap();
            assert!(!decoded.packet.is_empty());
            assert!(
                decoded
                    .packet
                    .iter()
                    .any(|packet| packet.track_descriptor.is_some())
            );
            assert!(!hub.recording());
            for number in 43..400 {
                hub.push_frame(super::super::FrameSample {
                    number,
                    ..Default::default()
                });
            }
            let (mut reconnect, _) = connect_async(&url).await.unwrap();
            call(
                &mut reconnect,
                1,
                "authenticate",
                json!({ "token": TOKEN, "version": 1 }),
            )
            .await;
            let reply = call(&mut reconnect, 2, "subscribe", json!({ "since": revision })).await;
            assert_eq!(reply["result"]["reset"], true);
            let retained = reply["result"]["frames"].as_array().unwrap();
            assert_eq!(retained.len(), 240);
            assert_eq!(retained.last().unwrap()["id"], 399);
            drop(server);
        });
    }
}
