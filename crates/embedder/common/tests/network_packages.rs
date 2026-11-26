#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "integration tests assert each step directly"
)]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use embedder::{DiagnosticsHub, NetKind};
use futures_util::future::{Either, select};
use futures_util::{SinkExt as _, StreamExt as _};
use javascript::host::{MethodExec, ModuleRegistry, PluginResult, Settle, Work};
use javascript::runtime::{ModuleCatalog, ModuleOutcome, Runtime, monotonic_ms};
use tokio::net::TcpListener as TokioTcpListener;
use tokio_tungstenite::tungstenite::protocol::Message;

struct TempBundle {
    dir: PathBuf,
}

impl TempBundle {
    fn pack(name: &str, entry_ts: &str) -> String {
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("impasto-net-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp test dir");
        let _guard = Self { dir: dir.clone() };

        let entry = dir.join(format!("{name}.ts"));
        fs::write(&entry, entry_ts).expect("write test entrypoint");

        bundler::bundle(bundler::BundleOptions {
            root: dir,
            entry,
            framework: root,
            ..Default::default()
        })
        .unwrap_or_else(|error| panic!("compile {name}: {error:#}"))
        .code
    }
}

impl Drop for TempBundle {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct TestExec;

impl MethodExec for TestExec {
    fn spawn(&self, task: javascript::host::HostFuture) {
        drop(tokio::runtime::Handle::current().spawn(task));
    }

    fn worker(&self, settle: Settle, work: Work) {
        std::thread::spawn(move || settle.send(work()));
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        settle.send(work());
    }

    fn now(&self, settle: Settle, result: PluginResult) {
        settle.send(result);
    }
}

fn run_bundled_module(bundled_js: &str, hub: Option<Arc<DiagnosticsHub>>) -> Result<(), String> {
    let worker = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    let _runtime = worker.enter();
    let mut context = impasto::PluginContext::default();
    if let Some(hub) = hub {
        context = context.with_diagnostics(hub);
    }
    let mut registrar = impasto::PluginRegistrar::default();
    impasto_http::register(&mut registrar);
    impasto_websocket::register(&mut registrar);
    let registry = ModuleRegistry::new(
        registrar
            .build(&context)
            .map_err(|error| error.to_string())?,
    );
    let catalog = ModuleCatalog::new(registry).map_err(|e| format!("{e:?}"))?;

    let (wake_tx, wake_rx) = mpsc::channel();
    let logs = Arc::new(Mutex::new(Vec::<String>::new()));
    let logs_ref = Arc::clone(&logs);

    let mut runtime = Runtime::builder()
        .modules(catalog)
        .executor(Arc::new(TestExec))
        .wake(move || {
            let _ = wake_tx.send(());
        })
        .log(move |level, line| {
            logs_ref
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(format!("[{level}] {line}"));
        })
        .module("app:main", bundled_js)
        .build()
        .map_err(|e| format!("build runtime: {e:?}"))?;

    let ticket = runtime
        .start_module("app:main")
        .map_err(|e| format!("start module: {e:?}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        let now = monotonic_ms();
        runtime
            .drive(now)
            .map_err(|e| format!("drive error: {e:?}"))?;

        if let Some(outcome) = runtime
            .take_module_result(ticket)
            .map_err(|e| format!("take result: {e:?}"))?
        {
            return match outcome {
                ModuleOutcome::Fulfilled => {
                    runtime.shutdown();
                    Ok(())
                }
                ModuleOutcome::Rejected(err) => {
                    let captured = logs.lock().unwrap().join("\n");
                    runtime.shutdown();
                    Err(format!("JS rejected: {err:?}\nlogs:\n{captured}"))
                }
            };
        }

        if Instant::now() >= deadline {
            let captured = logs.lock().unwrap().join("\n");
            runtime.shutdown();
            return Err(format!("timed out waiting for app:main\nlogs:\n{captured}"));
        }

        if runtime.jobs_pending() || runtime.due_timer_next_turn() {
            continue;
        }
        let wait = runtime
            .next_deadline_ms()
            .map(|due| {
                #[expect(
                    clippy::cast_sign_loss,
                    reason = "max(0) clamps negative deltas to zero before u64 cast"
                )]
                let delta = due.saturating_sub(monotonic_ms()).max(0) as u64;
                Duration::from_millis(delta).min(Duration::from_millis(20))
            })
            .unwrap_or(Duration::from_millis(20));
        let _ = wake_rx.recv_timeout(wait);
    }
}

fn read_http_request_raw(stream: &mut std::net::TcpStream) -> String {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if let Some(hdr_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header_str = String::from_utf8_lossy(&raw[..hdr_end]);
                    let content_len = header_str
                        .lines()
                        .find_map(|line| {
                            let (k, v) = line.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    if raw.len() >= hdr_end + 4 + content_len {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

fn request_path(raw: &str) -> &str {
    raw.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
}

#[test]
fn http_packages_redirects_abort_and_devtools_redaction() {
    let target_listener = TcpListener::bind("127.0.0.1:0").expect("bind target");
    let target_port = target_listener.local_addr().unwrap().port();
    let target_seen = Arc::new(Mutex::new(None::<String>));
    let target_seen_ref = Arc::clone(&target_seen);
    let target_server = std::thread::spawn(move || {
        let (mut stream, _) = target_listener.accept().expect("accept redirect target");
        let req = read_http_request_raw(&mut stream);
        *target_seen_ref.lock().unwrap() = Some(req.to_ascii_lowercase());
        let body = b"redirected";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.write_all(body);
    });

    let primary_listener = TcpListener::bind("127.0.0.1:0").expect("bind primary");
    let primary_port = primary_listener.local_addr().unwrap().port();
    let release_hang = Arc::new(AtomicBool::new(false));
    let release_hang_srv = Arc::clone(&release_hang);

    let primary_server = std::thread::spawn(move || {
        for _ in 0..6 {
            let Ok((mut stream, _)) = primary_listener.accept() else {
                break;
            };
            let raw = read_http_request_raw(&mut stream);
            match request_path(&raw) {
                "/post" => {
                    assert!(raw.contains(r#"{"hello":"world"}"#), "raw post:\n{raw}");
                    let body = br#"{"ok":true}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: sid=xyz\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.write_all(body);
                }
                "/chunked" => {
                    let resp = concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Transfer-Encoding: chunked\r\n",
                        "Connection: close\r\n\r\n",
                        "5\r\nfirst\r\n",
                        "7\r\n-second\r\n",
                        "0\r\n\r\n",
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
                "/redir" => {
                    let resp = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target_port}/done\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
                "/hang" => {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npart",
                    );
                    let _ = stream.flush();
                    let wait = Arc::clone(&release_hang_srv);
                    std::thread::spawn(move || {
                        let _held = stream;
                        while !wait.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    });
                }
                other => panic!("unexpected path {other}: {raw}"),
            }
        }
    });

    let source = format!(
        r#"
import {{ http }} from "@impasto/http";
import {{ fetch as pkgFetch }} from "@impasto/http/fetch";
import {{ installFetchPolyfill }} from "@impasto/http/polyfill";

installFetchPolyfill();
if (typeof fetch !== "function" || typeof Headers !== "function") {{
  throw new Error("polyfill did not install global fetch primitives");
}}

const postRes = await http.post("http://127.0.0.1:{primary_port}/post", {{
  headers: {{ Authorization: "Bearer top-secret" }},
  json: {{ hello: "world" }},
}});
const parsed = await postRes.json<{{ ok: boolean }}>();
if (!parsed.ok) throw new Error("unexpected JSON payload");

const streamRes = await http.get("http://127.0.0.1:{primary_port}/chunked");
const reader = streamRes.body!.getReader();
const parts: Uint8Array[] = [];
for (;;) {{
  const step = await reader.read();
  if (step.done) break;
  if (!(step.value instanceof Uint8Array)) {{
    throw new Error("stream chunk must normalize to Uint8Array");
  }}
  parts.push(step.value);
}}
const joined = parts.map((p) => new TextDecoder().decode(p)).join("");
if (joined !== "first-second") throw new Error("unexpected chunked body: " + joined);

const followed = await pkgFetch("http://127.0.0.1:{primary_port}/redir", {{
  headers: {{ Authorization: "Bearer secret-cross" }},
  redirect: "follow",
}});
if (!followed.redirected || followed.url !== "http://127.0.0.1:{target_port}/done") {{
  throw new Error("redirect follow metadata mismatch: " + followed.url);
}}
if ((await followed.text()) !== "redirected") throw new Error("redirected body mismatch");

const manual = await fetch("http://127.0.0.1:{primary_port}/redir", {{ redirect: "manual" }});
if (manual.status !== 302 || manual.redirected) throw new Error("manual redirect mismatch");

let errModeRejected = false;
try {{
  await fetch("http://127.0.0.1:{primary_port}/redir", {{ redirect: "error" }});
}} catch (err) {{
  errModeRejected = err instanceof TypeError;
}}
if (!errModeRejected) throw new Error("redirect: error did not reject with TypeError");

const ac = new AbortController();
const pending = http.get("http://127.0.0.1:{primary_port}/hang", {{ signal: ac.signal }});
setTimeout(() => ac.abort(), 30);
let abortedOk = false;
try {{
  const hangRes = await pending;
  await hangRes.text();
}} catch (err) {{
  abortedOk = err instanceof Error && err.name === "AbortError";
}}
if (!abortedOk) throw new Error("AbortController did not cancel in-flight HTTP request");
"#,
    );

    let bundled = TempBundle::pack("http_packages", &source);
    let hub = Arc::new(DiagnosticsHub::new());
    let result = run_bundled_module(&bundled, Some(Arc::clone(&hub)));
    release_hang.store(true, Ordering::Relaxed);
    result.expect("http integration script failed");

    let target_req = target_seen
        .lock()
        .unwrap()
        .take()
        .expect("redirect target hit");
    assert!(
        !target_req.contains("authorization:"),
        "cross-origin redirect leaked Authorization header:\n{target_req}"
    );

    let net_entries = hub.network_snapshot();
    assert!(net_entries.len() >= 4, "expected recorded HTTP calls");
    let post_entry = net_entries
        .iter()
        .find(|e| e.url.ends_with("/post"))
        .expect("/post entry recorded");
    assert_eq!(post_entry.status, Some(200));
    assert!(
        post_entry
            .request_headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "***"),
        "Authorization must be redacted in DevTools: {:?}",
        post_entry.request_headers
    );
    assert!(
        post_entry
            .response_headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("set-cookie") && v == "***"),
        "Set-Cookie must be redacted in DevTools: {:?}",
        post_entry.response_headers
    );

    let _ = target_server.join();
    let _ = primary_server.join();
}

#[test]
fn websocket_packages_compat_and_max_sockets() {
    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    let (addr, stop_tx) = rt.block_on(async {
        let listener = TokioTcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ws server");
        let addr = listener.local_addr().unwrap();
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            loop {
                let stop_fut = std::pin::pin!(stop_rx.changed());
                let accept_fut = std::pin::pin!(listener.accept());
                let accepted = match select(stop_fut, accept_fut).await {
                    Either::Left(_) => break,
                    Either::Right((res, _)) => res,
                };
                let Ok((stream, _)) = accepted else { break };
                tokio::spawn(async move {
                    #[expect(
                        clippy::result_large_err,
                        reason = "tokio-tungstenite accept_hdr_async callback uses ErrorResponse"
                    )]
                    let callback = |req: &http::Request<()>, mut resp: http::Response<()>| {
                        if req.headers().get("sec-websocket-protocol").is_some()
                            && let Ok(val) = http::HeaderValue::from_str("chat")
                        {
                            resp.headers_mut().insert("sec-websocket-protocol", val);
                        }
                        Ok(resp)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await
                    else {
                        return;
                    };
                    while let Some(Ok(msg)) = ws.next().await {
                        match msg {
                            Message::Text(txt) => {
                                let _ = ws.send(Message::Text(format!("echo:{txt}").into())).await;
                            }
                            Message::Binary(bin) => {
                                let _ = ws.send(Message::Binary(bin)).await;
                            }
                            Message::Close(frame) => {
                                let _ = ws.send(Message::Close(frame)).await;
                                let _ = ws.flush().await;
                                while ws.next().await.is_some() {}
                                break;
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        (addr, stop_tx)
    });

    let source = format!(
        r#"
import {{ connect }} from "@impasto/websocket";
import "@impasto/websocket/polyfill";

console.log("opening native WebSocket");
const ws = await connect("ws://{addr}/chat", {{ protocols: ["chat"] }});
if (ws.protocol !== "chat") throw new Error("expected negotiated protocol chat, got " + ws.protocol);
ws.send("ping");
ws.send(new Uint8Array([4, 5, 6]));

const got: Array<string | Uint8Array> = [];
const iter = ws.messages();
got.push((await iter.next()).value);
got.push((await iter.next()).value);
if (got[0] !== "echo:ping") throw new Error("text echo mismatch");
const bin = got[1] as Uint8Array;
if (bin.length !== 3 || bin[0] !== 4 || bin[2] !== 6) throw new Error("binary echo mismatch");
console.log("closing native WebSocket after both echoes");
const closeInfo = await ws.close(1000, "done");
if (!closeInfo.wasClean || closeInfo.code !== 1000) throw new Error("expected clean close");

// Verify WHATWG WebSocket polyfill + binary ArrayBuffer + events
console.log("exercising WebSocket polyfill");
await new Promise<void>((resolve, reject) => {{
  const compat = new WebSocket("ws://{addr}/compat");
  compat.binaryType = "arraybuffer";
  let seenText = false;
  let seenBuf = false;
  compat.addEventListener("open", () => {{
    compat.send("compat-hi");
    compat.send(new Uint8Array([9, 10]));
  }});
  compat.onmessage = (ev) => {{
    if (ev.data === "echo:compat-hi") seenText = true;
    else if (ev.data instanceof ArrayBuffer && new Uint8Array(ev.data)[0] === 9) seenBuf = true;
    if (seenText && seenBuf) compat.close(1000, "ok");
  }};
  compat.onclose = (ev) => {{
    if (seenText && seenBuf && ev.wasClean && ev.code === 1000) resolve();
    else reject(new Error("compat close state invalid"));
  }};
  compat.onerror = () => reject(new Error("unexpected compat error"));
}});

// Enforce MAX_SOCKETS = 8 and verify closing frees slots
console.log("filling WebSocket slots");
const pool = [];
for (let i = 0; i < 8; i++) {{
  pool.push(await connect("ws://{addr}/pool"));
}}
let ninthFailed = false;
try {{
  await connect("ws://{addr}/pool");
}} catch (error) {{
  if (!String(error).includes("too many open WebSockets")) throw error;
  ninthFailed = true;
}}
if (!ninthFailed) throw new Error("9th concurrent socket should fail MAX_SOCKETS=8");
console.log("replacing closed WebSockets at capacity");
for (let i = 0; i < pool.length; i++) {{
  const closed = await pool[i].close(1000, "free-slot");
  if (!closed.wasClean) throw new Error("expected clean slot close");
  pool[i] = await connect("ws://{addr}/pool");
}}
console.log("closing remaining WebSockets");
for (const socket of pool) {{
  await socket.close(1000, "bye");
}}
"#,
    );

    let bundled = TempBundle::pack("websocket_packages", &source);
    let hub = Arc::new(DiagnosticsHub::new());
    let outcome = run_bundled_module(&bundled, Some(Arc::clone(&hub)));
    let _ = stop_tx.send(true);

    let socket_entries: Vec<_> = hub
        .network_snapshot()
        .into_iter()
        .filter(|e| e.kind == NetKind::Socket)
        .collect();
    if let Err(error) = outcome {
        let events: Vec<_> = socket_entries
            .iter()
            .map(|event| (event.token, &event.method, &event.url, &event.error))
            .collect();
        panic!("WebSocket integration failed: {error}\nnative socket events: {events:?}");
    }
    assert!(
        !socket_entries.is_empty(),
        "expected WebSocket DevTools NetEntries"
    );
}

#[test]
fn slow_http_stream_does_not_block_concurrent_http_and_websocket() {
    let (slow_started_tx, slow_started_rx) = mpsc::channel();
    let (release_slow_tx, release_slow_rx) = mpsc::channel();

    let http_listener = TcpListener::bind("127.0.0.1:0").expect("bind http server");
    let http_port = http_listener.local_addr().unwrap().port();
    let http_server = std::thread::spawn(move || {
        let mut release_rx = Some(release_slow_rx);
        for _ in 0..2 {
            let Ok((mut stream, _)) = http_listener.accept() else {
                break;
            };
            let req = read_http_request_raw(&mut stream);
            let path = request_path(&req).to_owned();
            if path == "/slow" {
                let tx = slow_started_tx.clone();
                let rx = release_rx.take().expect("one slow request");
                std::thread::spawn(move || {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nslow",
                    );
                    let _ = stream.flush();
                    let _ = tx.send(());
                    let _ = rx.recv_timeout(Duration::from_secs(5));
                    let _ = stream.write_all(b"part");
                });
            } else if path == "/fast" {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfast",
                );
            }
        }
    });

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");
    let (ws_addr, ws_stop_tx) = rt.block_on(async {
        let listener = TokioTcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ws server");
        let addr = listener.local_addr().unwrap();
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            let stop_fut = std::pin::pin!(stop_rx.changed());
            let accept_fut = std::pin::pin!(listener.accept());
            if let Either::Right((Ok((stream, _)), _)) = select(stop_fut, accept_fut).await
                && let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await
            {
                while let Some(Ok(msg)) = ws.next().await {
                    if let Message::Text(t) = msg {
                        let _ = ws.send(Message::Text(t)).await;
                    } else if let Message::Close(f) = msg {
                        let _ = ws.send(Message::Close(f)).await;
                        let _ = ws.flush().await;
                        while ws.next().await.is_some() {}
                        break;
                    }
                }
            }
        });
        (addr, stop_tx)
    });

    // Gate inside JS: wait until /slow receives its first chunk, then verify /fast AND ws complete BEFORE /slow finishes!
    let barrier_listener = TcpListener::bind("127.0.0.1:0").expect("bind release gate");
    let barrier_port = barrier_listener.local_addr().unwrap().port();
    let release_gate = std::thread::spawn(move || {
        slow_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("slow stream started");
        let Ok((mut stream, _)) = barrier_listener.accept() else {
            return;
        };
        let _ = read_http_request_raw(&mut stream);
        let _ = release_slow_tx.send(());
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    });

    let source = format!(
        r#"
if (typeof (globalThis as Record<string, unknown>).fetch !== "undefined" ||
    typeof (globalThis as Record<string, unknown>).WebSocket !== "undefined") {{
  throw new Error("L1 runtime leaked network globals without polyfill");
}}
import {{ http }} from "@impasto/http";
import {{ connect }} from "@impasto/websocket";

const slowRes = await http.get("http://127.0.0.1:{http_port}/slow");
const slowReader = slowRes.body!.getReader();
const firstChunk = await slowReader.read();
if (firstChunk.done || new TextDecoder().decode(firstChunk.value) !== "slow") {{
  throw new Error("expected first slow chunk while slow server holds remaining bytes");
}}

// Slow stream is still mid-flight and holding its HTTP stream open; fast HTTP + WS must finish independently:
const fast = await http.get("http://127.0.0.1:{http_port}/fast");
if ((await fast.text()) !== "fast") throw new Error("fast HTTP failed");

const ws = await connect("ws://{ws_addr}/ws");
ws.send("hello");
const nextMsg = await ws.messages().next();
if (nextMsg.value !== "hello") throw new Error("concurrent WS failed");
await ws.close(1000, "done");

// Release the slow server's remaining bytes and read rest of slow stream:
await http.get("http://127.0.0.1:{barrier_port}/release");
const secondChunk = await slowReader.read();
if (secondChunk.done || new TextDecoder().decode(secondChunk.value) !== "part") {{
  throw new Error("expected trailing slow chunk");
}}
const eof = await slowReader.read();
if (!eof.done) throw new Error("expected slow stream EOF");
"#,
    );

    let bundled = TempBundle::pack("concurrency_packages", &source);
    let outcome = run_bundled_module(&bundled, None);
    let _ = ws_stop_tx.send(true);
    outcome.expect("concurrent stream/http/ws integration failed");

    let _ = release_gate.join();
    let _ = http_server.join();
}
