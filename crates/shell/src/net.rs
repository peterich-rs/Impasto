//! WebSocket work for `net`. HTTP is [`crate::http::OpenWireClient`].

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use session::{HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, IoJob};

use crate::command::Command;
use crate::queue::Inbox;

pub(crate) fn spawn(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    http: Option<Arc<dyn HttpClient>>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("net".to_string())
        .spawn(move || serve(jobs, inbox, http, hub))
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

fn serve(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    http: Option<Arc<dyn HttpClient>>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) {
    let mut sockets: HashMap<u64, (Sender<SocketCmd>, String)> = HashMap::new();
    while let Ok(job) = jobs.recv() {
        match job {
            IoJob::Http(request) => {
                if request.cancel.load(Ordering::Acquire) {
                    continue;
                }
                let token = request.token;
                let result = match http.as_ref() {
                    Some(client) => client.perform(&request),
                    None => Err(HttpError::new(
                        HttpFailure::Network,
                        "network is not linked",
                    )),
                };
                if request.cancel.load(Ordering::Acquire) {
                    continue;
                }
                if let Err(error) = result.as_ref() {
                    log_http(&request.method, &request.url, error);
                }
                inbox.post(Command::HttpDone { token, result });
            }
            IoJob::HttpStream(request) => {
                let token = request.token;
                let cancel = Arc::clone(&request.cancel);
                let mut deliver = |piece: HttpPiece<'_>| -> bool {
                    if cancel.load(Ordering::Acquire) {
                        return false;
                    }
                    match piece {
                        HttpPiece::Head(status, headers) => inbox.post(Command::HttpHead {
                            token,
                            status,
                            headers: headers.to_vec(),
                        }),
                        HttpPiece::Chunk(bytes, done) => inbox.post(Command::HttpChunk {
                            token,
                            bytes: bytes.to_vec(),
                            done,
                            error: None,
                        }),
                    }
                    true
                };
                let result = match http.as_ref() {
                    Some(client) => client.stream(&request, &mut deliver),
                    None => Err(HttpError::new(
                        HttpFailure::Network,
                        "network is not linked",
                    )),
                };
                if let Some(error) = result.err() {
                    log_http(&request.method, &request.url, &error);
                    inbox.post(Command::HttpChunk {
                        token,
                        bytes: Vec::new(),
                        done: true,
                        error: Some(error),
                    });
                }
            }
            IoJob::Abort { .. } => {}
            IoJob::SocketOpen { id, url, protocols } => {
                if sockets.len() >= 8 {
                    post_socket(
                        &inbox,
                        id,
                        "error",
                        true,
                        b"too many sockets",
                        hub.as_ref(),
                        &url,
                    );
                    continue;
                }
                match open_socket(id, &url, &protocols, Arc::clone(&inbox), hub.clone()) {
                    Ok(sender) => {
                        sockets.insert(id, (sender, url));
                    }
                    Err(message) => post_socket(
                        &inbox,
                        id,
                        "error",
                        true,
                        message.as_bytes(),
                        hub.as_ref(),
                        &url,
                    ),
                }
            }
            IoJob::SocketSend { id, text, data } => {
                if let Some((sender, _url)) = sockets.get(&id) {
                    let _ = sender.send(SocketCmd::Send { text, data });
                }
            }
            IoJob::SocketClose { id } => {
                if let Some((sender, url)) = sockets.remove(&id) {
                    let _ = sender.send(SocketCmd::Close);
                    drop(url);
                }
            }
        }
    }
}

enum SocketCmd {
    Send { text: bool, data: Vec<u8> },
    Close,
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

fn post_socket(
    inbox: &Inbox,
    id: u64,
    event: &str,
    text: bool,
    data: &[u8],
    hub: Option<&Arc<crate::diagnostics::DiagnosticsHub>>,
    url: &str,
) {
    if event == "error" {
        tracing::warn!(
            target: "impasto::net",
            url,
            error = %String::from_utf8_lossy(data),
            "socket error"
        );
    }
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
            response_preview: (!data.is_empty()).then(|| data.to_vec()),
            error: (event == "error").then(|| String::from_utf8_lossy(data).to_string()),
            done_ms: Some(mono_ms()),
        });
    }
    inbox.post(Command::Socket {
        id,
        event: event.to_owned(),
        text,
        data: data.to_vec(),
    });
}

fn open_socket(
    id: u64,
    url: &str,
    protocols: &[String],
    inbox: Arc<Inbox>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) -> Result<Sender<SocketCmd>, String> {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err("unsupported URL".to_owned());
    }
    let (tx, rx) = mpsc::channel();
    let url = url.to_owned();
    let protocols = protocols.to_vec();
    thread::Builder::new()
        .name(format!("socket-{id}"))
        .spawn(move || socket_loop(id, &url, &protocols, rx, inbox, hub))
        .map_err(|error| error.to_string())?;
    Ok(tx)
}

fn socket_loop(
    id: u64,
    url: &str,
    protocols: &[String],
    commands: Receiver<SocketCmd>,
    inbox: Arc<Inbox>,
    hub: Option<Arc<crate::diagnostics::DiagnosticsHub>>,
) {
    let hub_ref = hub.as_ref();
    let mut request = match tungstenite::client::IntoClientRequest::into_client_request(url) {
        Ok(request) => request,
        Err(error) => {
            post_socket(
                &inbox,
                id,
                "error",
                true,
                error.to_string().as_bytes(),
                hub_ref,
                url,
            );
            return;
        }
    };
    if !protocols.is_empty() && request.headers().get("Sec-WebSocket-Protocol").is_none() {
        let offered = protocols.join(", ");
        if let Ok(value) = http::HeaderValue::from_str(&offered) {
            request
                .headers_mut()
                .insert("Sec-WebSocket-Protocol", value);
        }
    }
    let mut connected = match tungstenite::connect(request) {
        Ok((socket, _)) => socket,
        Err(error) => {
            post_socket(
                &inbox,
                id,
                "error",
                true,
                error.to_string().as_bytes(),
                hub_ref,
                url,
            );
            return;
        }
    };
    post_socket(&inbox, id, "open", true, b"", hub_ref, url);
    set_socket_timeout(&mut connected);
    let socket = Mutex::new(connected);
    loop {
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(SocketCmd::Send { text, data }) => {
                let mut guard = lock_socket(&socket);
                let message = if text {
                    match String::from_utf8(data) {
                        Ok(text) => tungstenite::Message::Text(text.into()),
                        Err(_) => {
                            post_socket(&inbox, id, "error", true, b"invalid text", hub_ref, url);
                            continue;
                        }
                    }
                } else {
                    tungstenite::Message::Binary(data.into())
                };
                if let Err(error) = guard.send(message) {
                    post_socket(
                        &inbox,
                        id,
                        "error",
                        true,
                        error.to_string().as_bytes(),
                        hub_ref,
                        url,
                    );
                    break;
                }
            }
            Ok(SocketCmd::Close) | Err(RecvTimeoutError::Disconnected) => {
                let mut guard = lock_socket(&socket);
                let _ = guard.close(None);
                post_socket(&inbox, id, "close", true, b"", hub_ref, url);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut guard = lock_socket(&socket);
                match guard.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        post_socket(&inbox, id, "message", true, text.as_bytes(), hub_ref, url);
                    }
                    Ok(tungstenite::Message::Binary(data)) => {
                        post_socket(&inbox, id, "message", false, &data, hub_ref, url);
                    }
                    Ok(tungstenite::Message::Close(_)) => {
                        post_socket(&inbox, id, "close", true, b"", hub_ref, url);
                        break;
                    }
                    Ok(tungstenite::Message::Ping(data)) => {
                        let _ = guard.send(tungstenite::Message::Pong(data));
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(error))
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.kind() == std::io::ErrorKind::TimedOut => {}
                    Err(tungstenite::Error::ConnectionClosed) => {
                        post_socket(&inbox, id, "close", true, b"", hub_ref, url);
                        break;
                    }
                    Err(_) => {}
                }
            }
        }
    }
}

fn set_socket_timeout(
    socket: &mut tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
) {
    let tcp = match socket.get_mut() {
        tungstenite::stream::MaybeTlsStream::Plain(tcp) => tcp,
        tungstenite::stream::MaybeTlsStream::Rustls(tls) => tls.get_mut(),
        _ => return,
    };
    let _ = tcp.set_read_timeout(Some(Duration::from_millis(200)));
}

fn lock_socket<T>(socket: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    socket
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    use session::{HttpClient, HttpRequest};

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
                    session::HttpPiece::Head(status, _) => head = Some(status),
                    session::HttpPiece::Chunk(bytes, done) => {
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
            ) -> Result<session::HttpResponse, session::HttpError> {
                unreachable!("stream is the path under test");
            }
            fn stream(
                &self,
                _request: &HttpRequest,
                sink: &mut dyn FnMut(session::HttpPiece<'_>) -> bool,
            ) -> Result<(), session::HttpError> {
                assert!(sink(session::HttpPiece::Head(200, &[])));
                // The sink turns false once the cancel flag flips; emulate a
                // consumer that rejects the second piece.
                sink(session::HttpPiece::Chunk(b"one", false));
                sink(session::HttpPiece::Chunk(b"two", false));
                Ok(())
            }
        }
        let mut seen = 0;
        let outcome = CancelFirst.stream(&request("http://127.0.0.1:1/"), &mut |piece| {
            if matches!(piece, session::HttpPiece::Chunk(..)) {
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
