//! Loopback-only, authenticated DevTools transport. Never enters the app runtime.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
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

use super::TelemetryHub;
use super::hub::{CaptureOptions, TraceCapture};
use super::timeline::{FieldValue, TraceDetail};

const FRAME_SPAN_LIMIT: usize = 512;

const REQUEST_LIMIT: usize = 4096;
const CLIENT_LIMIT: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const UPDATE_INTERVAL: Duration = Duration::from_millis(250);
const VERSION: u8 = 1;

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
        Self::parse(
            &std::env::var("IMPASTO_DEVTOOLS_PORT").ok()?,
            &std::env::var("IMPASTO_DEVTOOLS_TOKEN").ok()?,
        )
    }
}

pub(crate) struct Server(JoinHandle<()>);

impl Server {
    pub(crate) fn start(config: Config, hub: Arc<TelemetryHub>) -> io::Result<Self> {
        let runtime = crate::rt::rt().map_err(io::Error::other)?.handle();
        let _enter = runtime.enter();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.port);
        let socket = std::net::TcpListener::bind(address)?;
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(socket)?;
        tracing::info!(target: "impasto::devtools", %address, "DevTools listening");
        Ok(Self(runtime.spawn(listen(listener, config.token, hub))))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn listen(listener: TcpListener, token: Arc<str>, hub: Arc<TelemetryHub>) {
    let mut clients = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        while clients.try_join_next().is_some() {}
        if clients.len() < CLIENT_LIMIT {
            clients.spawn(serve(stream, Arc::clone(&token), Arc::clone(&hub)));
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

async fn send(socket: &mut Socket, value: Value) -> bool {
    timeout(
        IO_TIMEOUT,
        socket.send(Message::Text(value.to_string().into())),
    )
    .await
    .is_ok_and(|result| result.is_ok())
}

fn response(id: Option<u64>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Option<u64>, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn matches_token(expected: &str, given: &str) -> bool {
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0, |different, (a, b)| different | (a ^ b))
            == 0
}

async fn serve(stream: TcpStream, token: Arc<str>, hub: Arc<TelemetryHub>) {
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
    if auth["jsonrpc"] != "2.0"
        || id.is_none()
        || auth["method"] != "authenticate"
        || auth["params"]["version"].as_u64() != Some(u64::from(VERSION))
        || !auth["params"]["token"]
            .as_str()
            .is_some_and(|given| matches_token(&token, given))
    {
        send(&mut socket, error(id, -32001, "authentication failed")).await;
        return;
    }
    if !send(&mut socket, response(id, json!({ "version": VERSION, "capabilities": ["frame_spans", "capture_status", "trace_detail", "capture_gpu"] }))).await {
        return;
    }
    let mut cursor = None;
    let mut captures = JoinSet::new();
    let mut updates = interval(UPDATE_INTERVAL);
    enum Next {
        Incoming(Option<Result<Message, SocketError>>),
        Finished(Option<Value>),
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
                request(&hub, &message, &mut cursor, &mut captures)
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
            Next::Update => cursor.and_then(|since| {
                let (revision, reset, samples) = hub.frame_updates(since);
                if revision == since {
                    return None;
                }
                cursor = Some(revision);
                Some(json!({
                    "jsonrpc": "2.0", "method": "frames",
                    "params": frame_update(revision, reset, &samples),
                }))
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
    message: &str,
    cursor: &mut Option<u64>,
    captures: &mut JoinSet<Value>,
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
                    Some(trace) => response(id, trace),
                    None => error(id, -32002, "trace capture unavailable"),
                }
            });
            return None;
        }
        _ => error(id, -32601, "unknown method"),
    })
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
        "topology": frame.schedule.topology.name(),
        "compositor_only": frame.schedule.compositor_only,
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
    fn subscription_backfills_gpu_and_exports_a_trace() {
        let _capture = super::super::hub::CAPTURE_TEST_LOCK.lock().unwrap();
        crate::rt::rt().unwrap().handle().block_on(async {
            let hub = Arc::new(TelemetryHub::new());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}/devtools", listener.local_addr().unwrap());
            let config = Config::parse("8765", TOKEN).unwrap();
            let server = Server(tokio::spawn(listen(
                listener,
                config.token,
                Arc::clone(&hub),
            )));
            let (mut incompatible, _) = connect_async(&url).await.unwrap();
            let denied = call(
                &mut incompatible,
                1,
                "authenticate",
                json!({
                    "token": TOKEN, "version": 2
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
