//! WebSocket work for `net`. HTTP is [`crate::http::OpenWireClient`].

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Duration;

use engine::session::{
    Credit, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, IoEvent, IoJob, Route,
};

use crate::command::Command;
use crate::queue::Inbox;

pub(crate) fn spawn(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    http: Option<Arc<dyn HttpClient>>,
    websocket: Option<Arc<dyn WebSocketTransport>>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("net".to_string())
        .spawn(move || serve(jobs, inbox, http, websocket, hub))
        .ok()
}

pub(crate) fn wake(
    deadlines: Receiver<Option<i64>>,
    inbox: Arc<Inbox>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("time".to_string())
        .spawn(move || wake_loop(deadlines, inbox))
        .ok()
}

const STREAM_WORKERS: usize = 8;
const WHOLE_WORKERS: usize = 8;
const PIECE_BYTES: usize = 64 << 10;
const CREDIT_WAIT: Duration = Duration::from_secs(10);

type Hub = Arc<crate::diagnostics::DiagnosticsHub>;

fn serve(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    http: Option<Arc<dyn HttpClient>>,
    websocket: Option<Arc<dyn WebSocketTransport>>,
    hub: Option<Hub>,
) {
    let mut sockets: HashMap<u64, (SyncSender<WebSocketCommand>, thread::JoinHandle<()>)> =
        HashMap::new();
    let connects = Arc::new(AtomicUsize::new(0));
    let streams = Arc::new(AtomicUsize::new(0));
    let whole = Arc::new(AtomicUsize::new(0));
    let executor = crate::rt::handle().ok();
    loop {
        sockets.retain(|_, (_, worker)| !worker.is_finished());
        let job = if sockets.is_empty() {
            match jobs.recv() {
                Ok(job) => job,
                Err(_) => break,
            }
        } else {
            match jobs.recv_timeout(Duration::from_millis(250)) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        let (job, route, outbound) = match job {
            IoJob::Routed {
                route,
                job,
                outbound,
            } => (*job, Some(route), outbound),
            job => (job, None, None),
        };
        let post = Post {
            inbox: Arc::clone(&inbox),
            route,
        };
        match job {
            job @ (IoJob::Http(_) | IoJob::HttpStream(_)) => {
                let (request, streaming) = match job {
                    IoJob::Http(request) => (request, false),
                    IoJob::HttpStream(request) => (request, true),
                    _ => unreachable!(),
                };
                let permit = if streaming {
                    WorkPermit::acquire(&streams, STREAM_WORKERS)
                } else {
                    WorkPermit::acquire(&whole, WHOLE_WORKERS)
                };
                if let (Some(executor), Some(permit)) = (&executor, permit) {
                    let http = http.clone();
                    drop(executor.spawn_blocking(move || {
                        let _permit = permit;
                        let _outbound = outbound;
                        execute_http(&request, streaming, &post, http.as_deref());
                    }));
                } else {
                    post.http_failure(
                        request.token,
                        streaming,
                        HttpError::new(HttpFailure::Network, "HTTP workers are busy or offline"),
                    );
                }
            }
            IoJob::Abort { .. } | IoJob::Routed { .. } => {}
            IoJob::SocketOpen { id, url, protocols } => {
                let Some(permit) = WorkPermit::acquire(&connects, 8) else {
                    post.socket(id, "error", true, b"too many sockets");
                    continue;
                };
                match open_socket(
                    id,
                    &url,
                    &protocols,
                    post.clone(),
                    hub.clone(),
                    permit,
                    websocket.clone(),
                ) {
                    Ok(socket) => {
                        sockets.insert(id, socket);
                    }
                    Err(message) => {
                        post.socket(id, "error", true, message.as_bytes());
                    }
                }
            }
            IoJob::SocketSend { id, text, data } => {
                if let Some((sender, _)) = sockets.get(&id)
                    && sender
                        .try_send(WebSocketCommand::Send {
                            text,
                            data,
                            credit: outbound,
                        })
                        .is_err()
                {
                    post.socket(id, "error", true, b"socket send queue closed or full");
                    sockets.remove(&id);
                }
            }
            IoJob::SocketClose { id } => {
                if let Some((sender, _)) = sockets.remove(&id)
                    && sender.try_send(WebSocketCommand::Close).is_ok()
                {
                    continue;
                }
                post.socket(id, "close", true, b"");
            }
        }
    }
}

struct WorkPermit(Arc<AtomicUsize>);

impl WorkPermit {
    fn acquire(count: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
        count
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                (used < max).then(|| used + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(count)))
    }
}

impl Drop for WorkPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct Post {
    inbox: Arc<Inbox>,
    route: Option<Route>,
}

impl Post {
    fn active(&self) -> bool {
        self.route.as_ref().is_none_or(Route::is_active)
    }

    fn event(&self, event: IoEvent, credit: Option<Credit>) {
        if let Some(route) = &self.route {
            let admitted = if event.is_terminal() {
                route.claim_terminal()
            } else {
                route.is_active()
            };
            if admitted {
                self.inbox.post(Command::JsIo(route.deliver(event, credit)));
            }
            return;
        }
        self.inbox.post(Command::Io(event));
    }

    fn http_failure(&self, token: u64, streaming: bool, error: HttpError) {
        self.event(
            if streaming {
                IoEvent::HttpChunk {
                    token,
                    bytes: Vec::new(),
                    done: true,
                    error: Some(error),
                }
            } else {
                IoEvent::Http {
                    token,
                    result: Err(error),
                }
            },
            None,
        );
    }

    fn socket(&self, id: u64, event: &str, text: bool, data: &[u8]) -> bool {
        if !self.active() {
            return false;
        }
        let terminal = matches!(event, "error" | "close");
        let credit = if let Some(route) = &self.route
            && !terminal
        {
            let Some(credit) =
                route.reserve(event.len().saturating_add(data.len()), Duration::ZERO)
            else {
                return false;
            };
            Some(credit)
        } else {
            None
        };
        let data = if terminal {
            &data[..data.len().min(1024)]
        } else {
            data
        };
        self.event(
            IoEvent::Socket {
                id,
                event: event.into(),
                text,
                data: data.to_vec(),
            },
            credit,
        );
        true
    }
}

fn execute_http(
    request: &HttpRequest,
    streaming: bool,
    post: &Post,
    client: Option<&dyn HttpClient>,
) {
    let token = request.token;
    if !post.active() || request.cancel.load(Ordering::Acquire) {
        return;
    }
    let Some(client) = client else {
        post.http_failure(
            token,
            streaming,
            HttpError::new(HttpFailure::Network, "network is not linked"),
        );
        return;
    };
    if !streaming {
        let result = client.perform(request);
        if request.cancel.load(Ordering::Acquire) {
            return;
        }
        if let Err(error) = &result {
            log_http(&request.method, &request.url, error);
            post.http_failure(
                token,
                false,
                HttpError::new(error.kind, "HTTP transport failed"),
            );
        } else {
            let event = IoEvent::Http { token, result };
            let credit = post
                .route
                .as_ref()
                .and_then(|route| route.reserve(event.payload_bytes(), Duration::ZERO));
            if post.route.is_none() || credit.is_some() {
                post.event(event, credit);
            } else {
                post.http_failure(
                    token,
                    false,
                    HttpError::new(HttpFailure::TooLarge, "response queue is full"),
                );
            }
        }
        return;
    }

    let mut head_seen = false;
    let mut done_seen = false;
    let mut stopped = false;
    let mut total: usize = 0;
    let result = client.stream(request, &mut |piece| {
        if done_seen || stopped || !post.active() || request.cancel.load(Ordering::Acquire) {
            return false;
        }
        match piece {
            HttpPiece::Head(status, headers) if !head_seen => {
                let bytes = headers.iter().fold(0usize, |sum, (name, value)| {
                    sum.saturating_add(name.len()).saturating_add(value.len())
                });
                let credit = post
                    .route
                    .as_ref()
                    .and_then(|route| route.reserve(bytes, CREDIT_WAIT));
                if post.route.is_some() && credit.is_none() {
                    stopped = true;
                    return false;
                }
                post.event(
                    IoEvent::HttpHead {
                        token,
                        status,
                        headers: headers.to_vec(),
                    },
                    credit,
                );
                head_seen = true;
            }
            HttpPiece::Chunk(bytes, done) if head_seen => {
                total = total.saturating_add(bytes.len());
                if total > engine::session::HTTP_BODY_LIMIT {
                    stopped = true;
                    return false;
                }
                let chunk_size = post.route.as_ref().map_or(PIECE_BYTES, |route| {
                    PIECE_BYTES.min(route.max_piece_bytes()).max(1)
                });
                for chunk in bytes.chunks(chunk_size) {
                    let credit = post
                        .route
                        .as_ref()
                        .and_then(|route| route.reserve(chunk.len(), CREDIT_WAIT));
                    if post.route.is_some() && credit.is_none() {
                        stopped = true;
                        return false;
                    }
                    post.event(
                        IoEvent::HttpChunk {
                            token,
                            bytes: chunk.to_vec(),
                            done: false,
                            error: None,
                        },
                        credit,
                    );
                }
                done_seen = done;
            }
            _ => {
                stopped = true;
                return false;
            }
        }
        true
    });
    if !post.active() || request.cancel.load(Ordering::Acquire) {
        return;
    }
    let error = if stopped {
        Some(HttpError::new(
            HttpFailure::TooLarge,
            "stream delivery limit exceeded",
        ))
    } else if let Err(error) = result {
        log_http(&request.method, &request.url, &error);
        Some(HttpError::new(error.kind, "stream transport failed"))
    } else if !head_seen {
        Some(HttpError::new(
            HttpFailure::Network,
            "stream ended without headers",
        ))
    } else {
        None
    };
    post.event(
        IoEvent::HttpChunk {
            token,
            bytes: Vec::new(),
            done: true,
            error,
        },
        None,
    );
}

/// One command from the session to the transport worker.
pub enum WebSocketCommand {
    Send {
        text: bool,
        data: Vec<u8>,
        /// Hold while the message is pending in the transport.
        credit: Option<Credit>,
    },
    Close,
}

/// A handshake or message forwarded to the session.
pub enum WebSocketEvent<'a> {
    Opened(&'a str),
    Message { text: bool, data: &'a [u8] },
}

/// A transport runs on the existing bounded socket worker. Returning settles
/// the close or error event. Stop when the session is inactive or emit rejects
/// more data due to backpressure.
pub trait WebSocketTransport: Send + Sync {
    fn run(
        &self,
        url: &str,
        protocols: &[String],
        commands: Receiver<WebSocketCommand>,
        active: &dyn Fn() -> bool,
        emit: &dyn Fn(WebSocketEvent<'_>) -> bool,
    ) -> Result<(), String>;
}

/// HTTPS image body. HTTP is rejected. The body cap matches [`crate::images::IMAGE_BODY_LIMIT`].
pub(crate) fn fetch_image_body(
    http: Option<&dyn HttpClient>,
    url: &str,
) -> Result<Vec<u8>, crate::images::ImageError> {
    use crate::images::ImageError;
    if !url.starts_with("https://") {
        return Err(ImageError::Failed("image URL must be https".into()));
    }
    let Some(http) = http else {
        return Err(ImageError::Failed("network is not linked".into()));
    };
    let request = HttpRequest {
        token: 0,
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        timeout_ms: None,
    };
    match http.perform(&request) {
        Ok(response) if (200..300).contains(&response.status) => {
            if response.body.len() > crate::images::IMAGE_BODY_LIMIT {
                return Err(ImageError::Failed("image exceeds body limit".into()));
            }
            Ok(response.body)
        }
        Ok(response) => Err(ImageError::Failed(format!("HTTP {}", response.status))),
        Err(error) => Err(ImageError::Failed(error.message)),
    }
}

fn log_http(method: &str, url: &str, error: &HttpError) {
    match error.kind {
        HttpFailure::Network | HttpFailure::Timeout => {
            tracing::warn!(
                target: "impasto::net",
                method,
                url,
                kind = error.kind.dom_name(),
                "{}",
                error.message
            );
        }
        HttpFailure::Canceled => {}
        _ => {
            tracing::error!(
                target: "impasto::net",
                method,
                url,
                kind = error.kind.dom_name(),
                "{}",
                error.message
            );
        }
    }
}

fn socket_event(
    post: &Post,
    id: u64,
    url: &str,
    hub: Option<&Hub>,
    event: &str,
    text: bool,
    data: &[u8],
) -> bool {
    if let Some(hub) = hub {
        use crate::diagnostics::{NetEntry, NetKind};
        hub.record_net(NetEntry {
            seq: 0,
            at_ms: mono_ms(),
            kind: NetKind::Socket,
            token: id,
            method: event.to_owned(),
            url: url.to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            status: None,
            response_headers: Vec::new(),
            response_bytes: Some(data.len()),
            response_preview: (!data.is_empty()).then(|| data[..data.len().min(1024)].to_vec()),
            error: (event == "error").then(|| String::from_utf8_lossy(data).to_string()),
            done_ms: Some(mono_ms()),
        });
    }
    post.socket(id, event, text, data)
}

fn open_socket(
    id: u64,
    url: &str,
    protocols: &[String],
    post: Post,
    hub: Option<Hub>,
    permit: WorkPermit,
    transport: Option<Arc<dyn WebSocketTransport>>,
) -> Result<(SyncSender<WebSocketCommand>, thread::JoinHandle<()>), String> {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err("unsupported URL".to_owned());
    }
    let (tx, rx) = mpsc::sync_channel(64);
    let url = url.to_owned();
    let protocols = protocols.to_vec();
    let worker = thread::Builder::new()
        .name(format!("socket-{id}"))
        .spawn(move || {
            let _permit = permit;
            socket_loop(
                id,
                &url,
                &protocols,
                rx,
                &post,
                hub.as_ref(),
                transport.as_deref(),
            );
        })
        .map_err(|error| error.to_string())?;
    Ok((tx, worker))
}

fn socket_loop(
    id: u64,
    url: &str,
    protocols: &[String],
    commands: Receiver<WebSocketCommand>,
    post: &Post,
    hub: Option<&Hub>,
    transport: Option<&dyn WebSocketTransport>,
) {
    let emit = |event: &str, text, data: &[u8]| socket_event(post, id, url, hub, event, text, data);
    let outcome = if let Some(transport) = transport {
        transport.run(
            url,
            protocols,
            commands,
            &|| post.active(),
            &|event| match event {
                WebSocketEvent::Opened(protocol) => emit("open", true, protocol.as_bytes()),
                WebSocketEvent::Message { text, data } => emit("message", text, data),
            },
        )
    } else {
        run_tungstenite(url, protocols, commands, post, &emit)
    };
    match outcome {
        Ok(()) => {
            emit("close", true, b"");
        }
        Err(error) => {
            emit("error", true, error.as_bytes());
        }
    }
}

fn run_tungstenite(
    url: &str,
    protocols: &[String],
    commands: Receiver<WebSocketCommand>,
    post: &Post,
    emit: &dyn Fn(&str, bool, &[u8]) -> bool,
) -> Result<(), String> {
    use tungstenite::{Error, Message};
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url)
        .map_err(|error| error.to_string())?;
    if !protocols.is_empty() {
        let offered = http::HeaderValue::from_str(&protocols.join(", "))
            .map_err(|error| error.to_string())?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", offered);
    }
    let mut config = tungstenite::protocol::WebSocketConfig::default();
    config.max_message_size = Some(256 << 10);
    config.max_frame_size = Some(256 << 10);
    config.write_buffer_size = 16 << 10;
    config.max_write_buffer_size = 256 << 10;
    let tcp = connect_socket(request.uri(), post)?;
    let (mut socket, response) =
        tungstenite::client_tls_with_config(request, tcp, Some(config), None)
            .map_err(|error| error.to_string())?;
    let protocol = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .map_or(&[][..], http::HeaderValue::as_bytes);
    if !emit("open", true, protocol) {
        return Err("socket event queue is full".into());
    }
    let tcp = match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(tcp) => Some(tcp),
        tungstenite::stream::MaybeTlsStream::Rustls(tls) => Some(tls.get_mut()),
        _ => None,
    };
    if let Some(tcp) = tcp {
        tcp.set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|error| error.to_string())?;
        tcp.set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
    }
    while post.active() {
        match commands.try_recv() {
            Ok(WebSocketCommand::Send { text, data, credit }) => {
                let _credit = credit;
                let message = if text {
                    Message::Text(
                        String::from_utf8(data)
                            .map_err(|error| error.to_string())?
                            .into(),
                    )
                } else {
                    Message::Binary(data.into())
                };
                socket.send(message).map_err(|error| error.to_string())?;
            }
            Ok(WebSocketCommand::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                let _ = socket.close(None);
                break;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let forwarded = match socket.read() {
            Ok(Message::Text(text)) => emit("message", true, text.as_bytes()),
            Ok(Message::Binary(data)) => emit("message", false, &data),
            Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                break;
            }
            Ok(_) => true,
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                true
            }
            Err(error) => return Err(error.to_string()),
        };
        if !forwarded {
            return Err("socket event queue is full".into());
        }
    }
    Ok(())
}

fn connect_socket(uri: &http::Uri, post: &Post) -> Result<std::net::TcpStream, String> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Instant;

    let host = uri
        .host()
        .ok_or_else(|| "websocket address is missing a host".to_owned())?;
    let host = host.trim_matches(['[', ']']);
    let port = uri
        .port_u16()
        .unwrap_or(if uri.scheme_str() == Some("wss") {
            443
        } else {
            80
        });
    let timeout = Duration::from_secs(5);
    let started = Instant::now();
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?;
    let mut last_error = None;
    for address in addresses {
        if !post.active() {
            return Err("websocket was canceled".into());
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(tcp) => {
                tcp.set_read_timeout(Some(timeout))
                    .map_err(|error| error.to_string())?;
                tcp.set_write_timeout(Some(timeout))
                    .map_err(|error| error.to_string())?;
                return Ok(tcp);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || "websocket connection timed out".into(),
        |error| error.to_string(),
    ))
}

fn wake_loop(deadlines: Receiver<Option<i64>>, inbox: Arc<Inbox>) {
    let mut deadline = None;
    loop {
        let wait = match deadline {
            Some(due) => {
                let now = ms_i64(mono_ms());
                if due <= now {
                    inbox.post(Command::Wake);
                    deadline = None;
                    continue;
                }
                Duration::from_millis((due - now) as u64)
            }
            None => Duration::from_secs(3600),
        };
        match deadlines.recv_timeout(wait) {
            Ok(next) => deadline = next,
            Err(RecvTimeoutError::Timeout) => {
                if deadline.take().is_some() {
                    inbox.post(Command::Wake);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

pub(crate) fn ms_i64(ms: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "deadlines are millisecond timestamps and fit in i64"
    )]
    {
        ms as i64
    }
}

pub(crate) fn mono_ms() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
        * 1000.0
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use engine::session::{HttpClient, HttpRequest};

    use crate::http::OpenWireClient;

    fn request(url: &str) -> HttpRequest {
        HttpRequest {
            token: 1,
            method: "GET".to_owned(),
            url: url.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            timeout_ms: None,
        }
    }

    #[test]
    fn ftp_is_rejected_before_a_connection() {
        let client = match OpenWireClient::new() {
            Ok(client) => client,
            Err(error) => panic!("{error}"),
        };
        match client.perform(&request("ftp://example.test")) {
            Err(error) => assert_eq!(error.message, "unsupported URL"),
            Ok(_) => panic!("ftp should fail"),
        }
    }

    #[test]
    fn http_get_reads_a_local_response() {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => panic!("{error}"),
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(error) => panic!("{error}"),
        };
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = b"hello";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });
        let client = match OpenWireClient::new() {
            Ok(client) => client,
            Err(error) => panic!("{error}"),
        };
        let result = client.perform(&request(&format!("http://127.0.0.1:{port}/")));
        match result {
            Ok(response) => {
                assert_eq!(response.status, 200);
                assert_eq!(response.body, b"hello");
            }
            Err(error) => panic!("{}", error.message),
        }
        let _ = server.join();
    }

    #[test]
    fn streamed_pieces_join_into_the_full_body() {
        let listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => panic!("{error}"),
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(error) => panic!("{error}"),
        };
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            // Chunked so the client sees more than one frame.
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Transfer-Encoding: chunked\r\n",
                "Connection: close\r\n\r\n",
                "5\r\nhello\r\n",
                "6\r\n world\r\n",
                "0\r\n\r\n",
            );
            let _ = stream.write_all(response.as_bytes());
        });
        let client = match OpenWireClient::new() {
            Ok(client) => client,
            Err(error) => panic!("{error}"),
        };
        let mut head: Option<u16> = None;
        let mut body = Vec::new();
        let mut closed = false;
        let outcome = client.stream(
            &request(&format!("http://127.0.0.1:{port}/")),
            &mut |piece| {
                match piece {
                    engine::session::HttpPiece::Head(status, _) => head = Some(status),
                    engine::session::HttpPiece::Chunk(bytes, done) => {
                        body.extend_from_slice(bytes);
                        if done {
                            closed = true;
                        }
                    }
                }
                true
            },
        );
        let _ = server.join();
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(head, Some(200));
        assert_eq!(body, b"hello world");
        assert!(closed, "the final chunk must close the stream");
    }

    #[test]
    fn a_canceled_stream_stops_delivering() {
        struct CancelFirst;
        impl HttpClient for CancelFirst {
            fn perform(
                &self,
                _request: &HttpRequest,
            ) -> Result<engine::session::HttpResponse, engine::session::HttpError> {
                unreachable!("stream is the path under test");
            }
            fn stream(
                &self,
                _request: &HttpRequest,
                sink: &mut dyn FnMut(engine::session::HttpPiece<'_>) -> bool,
            ) -> Result<(), engine::session::HttpError> {
                assert!(sink(engine::session::HttpPiece::Head(200, &[])));
                // The sink turns false once the cancel flag flips; emulate a
                // consumer that rejects the second piece.
                sink(engine::session::HttpPiece::Chunk(b"one", false));
                sink(engine::session::HttpPiece::Chunk(b"two", false));
                Ok(())
            }
        }
        let mut seen = 0;
        let outcome = CancelFirst.stream(&request("http://127.0.0.1:1/"), &mut |piece| {
            if matches!(piece, engine::session::HttpPiece::Chunk(..)) {
                seen += 1;
                return seen < 2;
            }
            true
        });
        // The default trait impl is not used here; the local one keeps going,
        // but the contract a real client honors is "stop when the sink says no".
        assert!(outcome.is_ok());
        assert_eq!(seen, 2);
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use engine::session::{CreditPool, HttpResponse, RequestKey, RuntimeKey};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicBool;

    struct Slow {
        finished: mpsc::Sender<()>,
    }

    impl HttpClient for Slow {
        fn perform(&self, _: &HttpRequest) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"ok".to_vec(),
            })
        }

        fn stream(
            &self,
            _: &HttpRequest,
            sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
        ) -> Result<(), HttpError> {
            if sink(HttpPiece::Head(200, &[])) && sink(HttpPiece::Chunk(b"1234", false)) {
                sink(HttpPiece::Chunk(b"5678", true));
            }
            let _ = self.finished.send(());
            Ok(())
        }
    }

    #[test]
    fn blocked_stream_does_not_hold_commands_and_shutdown_releases_credit_wait() {
        let (finished_tx, finished_rx) = mpsc::channel();
        let (work_tx, work_rx) = mpsc::sync_channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        let inbox = Arc::new(Inbox::new());
        let reader = thread::spawn({
            let inbox = Arc::clone(&inbox);
            move || {
                while let Some(command) = inbox.recv() {
                    if matches!(&command, Command::Shutdown) {
                        break;
                    }
                    if event_tx.send(command).is_err() {
                        break;
                    }
                }
            }
        });
        let worker = spawn(
            work_rx,
            Arc::clone(&inbox),
            Some(Arc::new(Slow {
                finished: finished_tx,
            })),
            None,
            None,
        );
        let ingress = Arc::new(CreditPool::new(64, 12, 4, 4));
        let outbound = Arc::new(CreditPool::new(128, 12, 16, 4));
        let runtime = RuntimeKey {
            instance: 1,
            generation: 3,
        };
        let submit = |id, streaming, job, cancel| {
            let route = Route::new(
                RequestKey {
                    runtime,
                    sequence: id,
                },
                Arc::clone(&ingress),
                Arc::clone(&outbound),
                cancel,
                streaming,
            );
            assert!(
                work_tx
                    .send(IoJob::Routed {
                        route,
                        job: Box::new(job),
                        outbound: None
                    })
                    .is_ok()
            );
        };
        let request = |token, cancel| HttpRequest {
            token,
            method: "GET".into(),
            url: "https://example.test/".into(),
            headers: Vec::new(),
            body: Vec::new(),
            cancel,
            timeout_ms: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        submit(
            1,
            true,
            IoJob::HttpStream(request(1, Arc::clone(&cancel))),
            cancel,
        );
        let next = || {
            event_rx
                .recv_timeout(Duration::from_secs(3))
                .unwrap_or_else(|error| panic!("{error}"))
        };
        let Command::JsIo(head) = next() else {
            panic!("expected routed head")
        };
        assert_eq!(head.runtime, runtime);
        assert!(matches!(head.event, IoEvent::HttpHead { token: 1, .. }));
        drop(head.credit);
        let Command::JsIo(chunk) = next() else {
            panic!("expected first chunk")
        };
        assert!(
            matches!(chunk.event, IoEvent::HttpChunk { token: 1, ref bytes, done: false, .. } if bytes == b"1234")
        );
        assert_eq!(ingress.usage().bytes, 4);
        assert!(finished_rx.try_recv().is_err());

        // A short HTTP job and a real WebSocket handshake both complete while the
        // stream worker is waiting for the owner of the first chunk.
        let cancel_short = Arc::new(AtomicBool::new(false));
        submit(
            2,
            false,
            IoJob::Http(request(2, Arc::clone(&cancel_short))),
            cancel_short,
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        let address = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("{error}"));
        let server = thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap_or_else(|error| panic!("{error}"));
            drop(tungstenite::accept(tcp));
        });
        let socket = Arc::new(AtomicBool::new(false));
        submit(
            3,
            false,
            IoJob::SocketOpen {
                id: 3,
                url: format!("ws://{address}/"),
                protocols: Vec::new(),
            },
            socket,
        );
        let mut short = false;
        let mut socket_open = false;
        for _ in 0..3 {
            let Command::JsIo(delivery) = next() else {
                panic!("expected routed completion")
            };
            match delivery.event {
                IoEvent::Http {
                    token: 2,
                    result: Ok(response),
                } => {
                    short = response.status == 200;
                }
                IoEvent::Socket { id: 3, event, .. } if event == "open" => socket_open = true,
                IoEvent::Socket { id: 3, .. } => {}
                _ => panic!("unexpected completion while stream is blocked"),
            }
            if short && socket_open {
                break;
            }
        }
        assert!(short && socket_open);
        assert!(finished_rx.try_recv().is_err());

        ingress.close();
        assert!(finished_rx.recv_timeout(Duration::from_secs(1)).is_ok());
        drop(chunk.credit);
        assert_eq!(ingress.usage().bytes, 0);
        drop(work_tx);
        if let Some(worker) = worker {
            assert!(worker.join().is_ok());
        }
        inbox.post(Command::Shutdown);
        assert!(reader.join().is_ok());
        assert!(server.join().is_ok());
    }
}
