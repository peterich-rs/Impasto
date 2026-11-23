//! Loopback-only, authenticated DevTools transport. Never enters the app runtime.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async_with_config,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        protocol::WebSocketConfig,
    },
};

use super::TelemetryHub;

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
    if !send(&mut socket, response(id, json!({ "version": VERSION }))).await {
        return;
    }
    let mut cursor = None;
    loop {
        let message = match timeout(UPDATE_INTERVAL, socket.next()).await {
            Ok(Some(Ok(Message::Text(message)))) => message,
            Ok(Some(Ok(Message::Ping(data)))) => {
                if timeout(IO_TIMEOUT, socket.send(Message::Pong(data)))
                    .await
                    .is_ok_and(|result| result.is_ok())
                {
                    continue;
                }
                break;
            }
            Ok(Some(Ok(Message::Pong(_)))) => continue,
            Ok(_) => break,
            Err(_) => {
                if let Some(since) = cursor {
                    let (revision, reset, samples) = hub.frame_updates(since);
                    if revision != since {
                        let update = json!({
                            "jsonrpc": "2.0", "method": "frames",
                            "params": frame_update(revision, reset, &samples),
                        });
                        if !send(&mut socket, update).await {
                            break;
                        }
                        cursor = Some(revision);
                    }
                }
                continue;
            }
        };
        let result = request(&hub, &message, &mut cursor).await;
        if !send(&mut socket, result).await {
            break;
        }
    }
}

async fn request(hub: &TelemetryHub, message: &str, cursor: &mut Option<u64>) -> Value {
    let Ok(rpc) = serde_json::from_str::<Value>(message) else {
        return error(None, -32700, "invalid JSON");
    };
    let id = rpc["id"].as_u64();
    if rpc["jsonrpc"] != "2.0" || id.is_none() || !rpc["method"].is_string() {
        return error(id, -32600, "invalid request");
    }
    match rpc["method"].as_str() {
        Some("subscribe") => {
            let since = rpc["params"]["since"].as_u64().unwrap_or_default();
            let (revision, reset, samples) = hub.frame_updates(since);
            *cursor = Some(revision);
            response(id, frame_update(revision, reset, &samples))
        }
        Some("capture") => {
            let duration = rpc["params"]["duration_ms"].as_u64().unwrap_or(5000);
            if !(100..=5000).contains(&duration) {
                return error(id, -32602, "capture must run for 100 to 5000 milliseconds");
            }
            match capture(hub, Duration::from_millis(duration)).await {
                Some(trace) => response(id, trace),
                None => error(id, -32002, "trace capture unavailable"),
            }
        }
        _ => error(id, -32601, "unknown method"),
    }
}

struct Capture<'a> {
    hub: &'a TelemetryHub,
    id: u64,
}

impl Drop for Capture<'_> {
    fn drop(&mut self) {
        self.hub.take_capture(self.id);
    }
}

async fn capture(hub: &TelemetryHub, duration: Duration) -> Option<Value> {
    let capture = Capture {
        hub,
        id: hub.begin_capture(None)?,
    };
    sleep(duration).await;
    let archive = hub.take_capture(capture.id)?;
    tokio::task::spawn_blocking(move || super::trace_export::document(&archive))
        .await
        .ok()
}

fn frame_update(revision: u64, reset: bool, samples: &[super::FrameSample]) -> Value {
    let frames: Vec<_> = samples
        .iter()
        .map(|frame| {
            let render = frame.raster.render;
            let passes = frame.gpu_timing.as_ref().map(|gpu| {
                gpu.passes
                    .iter()
                    .map(|pass| {
                        json!({
                            "kind": format!("{:?}", pass.kind), "slice": pass.slice_index,
                            "draws": pass.draws, "start_us": pass.start_us, "duration_us": pass.duration_us,
                        })
                    })
                    .collect::<Vec<_>>()
            });
            json!({
                "id": frame.number, "revision": frame.changed_at, "budget_ms": frame.budget_ms,
                "ui_ms": frame.schedule.ui_ms(), "raster_ms": frame.schedule.raster_ms(),
                "queue_ms": frame.schedule.queue_ms(), "e2e_ms": frame.schedule.e2e_ms(),
                "over_budget": frame.over_budget, "draws": render.draws, "passes": render.passes,
                "gpu": {
                    "state": frame.gpu.status.name(), "generation": frame.gpu.device_generation,
                    "frame_span_us": frame.gpu_timing.as_ref().map(|gpu| gpu.frame_span_us),
                    "passes_sum_us": frame.gpu_timing.as_ref().map(|gpu| gpu.passes_sum_us), "passes": passes,
                },
            })
        })
        .collect();
    json!({ "cursor": revision, "reset": reset, "frames": frames })
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

            let reply = call(&mut client, 4, "capture", json!({ "duration_ms": 100 })).await;
            assert_eq!(reply["result"]["traceEvents"].as_array().unwrap().len(), 5);
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
