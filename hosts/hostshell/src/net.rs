//! Blocking HTTP and WebSocket work for `uifw-net`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use session::{
    HTTP_BODY_LIMIT, HttpError, HttpRequest, HttpResponse, IoJob, PluginResult, PluginValue,
};

use crate::command::Command;
use crate::platform::PlatformBridge;
use crate::queue::Inbox;

pub(crate) fn spawn(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    bridge: Arc<dyn PlatformBridge>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("uifw-net".to_string())
        .spawn(move || serve(jobs, inbox, bridge))
        .ok()
}

pub(crate) fn wake(
    deadlines: Receiver<Option<i64>>,
    inbox: Arc<Inbox>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("uifw-time".to_string())
        .spawn(move || wake_loop(deadlines, inbox))
        .ok()
}

fn serve(jobs: Receiver<IoJob>, inbox: Arc<Inbox>, bridge: Arc<dyn PlatformBridge>) {
    let mut sockets: HashMap<u64, Sender<SocketCmd>> = HashMap::new();
    while let Ok(job) = jobs.recv() {
        match job {
            IoJob::Http(request) => {
                if request.cancel.load(Ordering::Acquire) {
                    continue;
                }
                let token = request.token;
                let result = perform_http(&request);
                if request.cancel.load(Ordering::Acquire) {
                    continue;
                }
                inbox.post(Command::HttpDone { token, result });
            }
            IoJob::Abort { .. } => {}
            IoJob::Plugin {
                token,
                name,
                method,
                args,
            } => {
                let result = plugin(&bridge, &name, &method, &args);
                inbox.post(Command::PluginDone { token, result });
            }
            IoJob::SocketOpen { id, url } => {
                if sockets.len() >= 8 {
                    post_socket(&inbox, id, "error", true, b"too many sockets");
                    continue;
                }
                match open_socket(id, &url, Arc::clone(&inbox)) {
                    Ok(sender) => {
                        sockets.insert(id, sender);
                    }
                    Err(message) => post_socket(&inbox, id, "error", true, message.as_bytes()),
                }
            }
            IoJob::SocketSend { id, text, data } => {
                if let Some(sender) = sockets.get(&id) {
                    let _ = sender.send(SocketCmd::Send { text, data });
                }
            }
            IoJob::SocketClose { id } => {
                if let Some(sender) = sockets.remove(&id) {
                    let _ = sender.send(SocketCmd::Close);
                }
            }
        }
    }
}

enum SocketCmd {
    Send { text: bool, data: Vec<u8> },
    Close,
}

/// HTTPS image body. HTTP is rejected. The body cap matches [`session::HTTP_BODY_LIMIT`].
pub(crate) fn fetch_image_body(url: &str) -> Result<Vec<u8>, crate::images::ImageError> {
    use crate::images::ImageError;
    if !url.starts_with("https://") {
        return Err(ImageError::Failed("image URL must be https".into()));
    }
    let request = HttpRequest {
        token: 0,
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: Vec::new(),
        body: Vec::new(),
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    match perform_http(&request) {
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

pub(crate) fn perform_http(request: &HttpRequest) -> Result<HttpResponse, HttpError> {
    if !(request.url.starts_with("http://") || request.url.starts_with("https://")) {
        return Err(HttpError::new("unsupported URL"));
    }
    let mut url = request.url.clone();
    let mut method = request.method.clone();
    let mut body = request.body.clone();
    for _ in 0..5 {
        let target = parse_http_url(&url)?;
        let response = exchange(&target, &method, &request.headers, &body)?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(location) = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.clone())
        else {
            return Ok(response);
        };
        url = resolve_redirect(&target, &location);
        if response.status == 303 {
            method = "GET".to_owned();
            body.clear();
        }
    }
    Err(HttpError::new("too many redirects"))
}

struct HttpTarget {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<HttpTarget, HttpError> {
    let (https, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(HttpError::new("unsupported URL"));
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return Err(HttpError::new("unsupported URL"));
    }
    let (host, port) = if let Some(host) = authority.strip_prefix('[') {
        let Some((host, port)) = host.split_once("]:") else {
            return Err(HttpError::new("unsupported URL"));
        };
        let port = port
            .parse::<u16>()
            .map_err(|_| HttpError::new("unsupported URL"))?;
        (host.to_owned(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|_| HttpError::new("unsupported URL"))?;
        (host.to_owned(), port)
    } else {
        (authority.to_owned(), if https { 443 } else { 80 })
    };
    let path = if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path}")
    };
    Ok(HttpTarget {
        https,
        host,
        port,
        path,
    })
}

fn resolve_redirect(current: &HttpTarget, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_owned();
    }
    let scheme = if current.https { "https" } else { "http" };
    if let Some(path) = location.strip_prefix('/') {
        return format!("{scheme}://{}:{}/{path}", current.host, current.port);
    }
    format!(
        "{scheme}://{}:{}{}",
        current.host, current.port, current.path
    )
}

fn exchange(
    target: &HttpTarget,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<HttpResponse, HttpError> {
    let address = format!("{}:{}", target.host, target.port);
    let mut tcp =
        std::net::TcpStream::connect(address).map_err(|error| HttpError::new(error.to_string()))?;
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| HttpError::new(error.to_string()))?;
    tcp.set_write_timeout(Some(Duration::from_secs(20)))
        .map_err(|error| HttpError::new(error.to_string()))?;
    if target.https {
        let mut tls = tls_stream(tcp, &target.host)?;
        write_request(&mut tls, target, method, headers, body)?;
        read_http(&mut tls)
    } else {
        write_request(&mut tcp, target, method, headers, body)?;
        read_http(&mut tcp)
    }
}

fn tls_stream(
    tcp: std::net::TcpStream,
    host: &str,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>, HttpError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| HttpError::new("unsupported URL"))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), name)
        .map_err(|error| HttpError::new(error.to_string()))?;
    Ok(rustls::StreamOwned::new(connection, tcp))
}

fn write_request(
    stream: &mut dyn Write,
    target: &HttpTarget,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<(), HttpError> {
    let mut head = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        target.path,
        target.host,
        body.len()
    );
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| HttpError::new(error.to_string()))
}

fn read_http(stream: &mut dyn Read) -> Result<HttpResponse, HttpError> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    while raw.windows(4).all(|window| window != b"\r\n\r\n") {
        let count = read_some(stream, &mut buf)?;
        if count == 0 {
            return Err(HttpError::new("short response"));
        }
        raw.extend_from_slice(&buf[..count]);
        if raw.len() > HTTP_BODY_LIMIT {
            return Err(HttpError::new("TypeError"));
        }
    }
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| HttpError::new("short response"))?
        + 4;
    let mut header_slots = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Response::new(&mut header_slots);
    let status = parsed
        .parse(&raw[..split])
        .map_err(|error| HttpError::new(error.to_string()))?;
    if status.is_partial() {
        return Err(HttpError::new("short response"));
    }
    let Some(code) = parsed.code else {
        return Err(HttpError::new("short response"));
    };
    let headers = parsed
        .headers
        .iter()
        .map(|header| {
            (
                header.name.to_owned(),
                String::from_utf8_lossy(header.value).into_owned(),
            )
        })
        .collect::<Vec<_>>();
    let mut payload = raw[split..].to_vec();
    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
    });
    let length = headers.iter().find_map(|(name, value)| {
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.parse::<usize>().ok())
            .flatten()
    });
    if let Some(length) = length {
        while payload.len() < length {
            let count = read_some(stream, &mut buf)?;
            if count == 0 {
                break;
            }
            payload.extend_from_slice(&buf[..count]);
            if payload.len() > HTTP_BODY_LIMIT {
                return Err(HttpError::new("TypeError"));
            }
        }
        payload.truncate(length);
    } else if chunked {
        loop {
            let count = read_some(stream, &mut buf)?;
            if count == 0 {
                break;
            }
            payload.extend_from_slice(&buf[..count]);
            if payload.len() > HTTP_BODY_LIMIT {
                return Err(HttpError::new("TypeError"));
            }
        }
        payload = decode_chunked(&payload)?;
    } else {
        loop {
            let count = read_some(stream, &mut buf)?;
            if count == 0 {
                break;
            }
            payload.extend_from_slice(&buf[..count]);
            if payload.len() > HTTP_BODY_LIMIT {
                return Err(HttpError::new("TypeError"));
            }
        }
    }
    if payload.len() > HTTP_BODY_LIMIT {
        return Err(HttpError::new("TypeError"));
    }
    Ok(HttpResponse {
        status: code,
        headers,
        body: payload,
    })
}

fn read_some(stream: &mut dyn Read, buf: &mut [u8]) -> Result<usize, HttpError> {
    loop {
        match stream.read(buf) {
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(HttpError::new(error.to_string())),
        }
    }
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    let mut rest = data;
    loop {
        let Some(end) = rest.windows(2).position(|window| window == b"\r\n") else {
            return Err(HttpError::new("chunk"));
        };
        let line = std::str::from_utf8(&rest[..end]).map_err(|_| HttpError::new("chunk"))?;
        let size = usize::from_str_radix(line.split(';').next().unwrap_or(line).trim(), 16)
            .map_err(|_| HttpError::new("chunk"))?;
        rest = &rest[end + 2..];
        if size == 0 {
            break;
        }
        if rest.len() < size + 2 {
            return Err(HttpError::new("chunk"));
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
        if out.len() > HTTP_BODY_LIMIT {
            return Err(HttpError::new("TypeError"));
        }
    }
    Ok(out)
}

fn plugin(
    bridge: &Arc<dyn PlatformBridge>,
    name: &str,
    method: &str,
    args: &[PluginValue],
) -> PluginResult {
    let text = match args.first() {
        Some(PluginValue::Str(value)) => value.as_str(),
        _ => "",
    };
    match (name, method) {
        ("Clipboard", "getString") => PluginResult::Done(PluginValue::Str(bridge.read_clipboard())),
        ("Clipboard", "setString") => {
            bridge.write_clipboard(text);
            PluginResult::Done(PluginValue::Null)
        }
        ("Linking", "openURL") => match bridge.open_url(text) {
            Ok(()) => PluginResult::Done(PluginValue::Null),
            Err(message) => PluginResult::Failed { message },
        },
        _ => PluginResult::Failed {
            message: "module is not linked".to_owned(),
        },
    }
}

fn post_socket(inbox: &Inbox, id: u64, event: &str, text: bool, data: &[u8]) {
    inbox.post(Command::Socket {
        id,
        event: event.to_owned(),
        text,
        data: data.to_vec(),
    });
}

fn open_socket(id: u64, url: &str, inbox: Arc<Inbox>) -> Result<Sender<SocketCmd>, String> {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err("unsupported URL".to_owned());
    }
    let (tx, rx) = mpsc::channel();
    let url = url.to_owned();
    thread::Builder::new()
        .name(format!("uifw-socket-{id}"))
        .spawn(move || socket_loop(id, &url, rx, inbox))
        .map_err(|error| error.to_string())?;
    Ok(tx)
}

fn socket_loop(id: u64, url: &str, commands: Receiver<SocketCmd>, inbox: Arc<Inbox>) {
    let mut connected = match tungstenite::connect(url) {
        Ok((socket, _)) => socket,
        Err(error) => {
            post_socket(&inbox, id, "error", true, error.to_string().as_bytes());
            return;
        }
    };
    post_socket(&inbox, id, "open", true, b"");
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
                            post_socket(&inbox, id, "error", true, b"invalid text");
                            continue;
                        }
                    }
                } else {
                    tungstenite::Message::Binary(data.into())
                };
                if let Err(error) = guard.send(message) {
                    post_socket(&inbox, id, "error", true, error.to_string().as_bytes());
                    break;
                }
            }
            Ok(SocketCmd::Close) | Err(RecvTimeoutError::Disconnected) => {
                let mut guard = lock_socket(&socket);
                let _ = guard.close(None);
                post_socket(&inbox, id, "close", true, b"");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                let mut guard = lock_socket(&socket);
                match guard.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        post_socket(&inbox, id, "message", true, text.as_bytes());
                    }
                    Ok(tungstenite::Message::Binary(data)) => {
                        post_socket(&inbox, id, "message", false, &data);
                    }
                    Ok(tungstenite::Message::Close(_)) => {
                        post_socket(&inbox, id, "close", true, b"");
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
                        post_socket(&inbox, id, "close", true, b"");
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
                let now = mono_ms() as i64;
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

    use session::HttpRequest;

    use super::perform_http;

    fn request(url: &str) -> HttpRequest {
        HttpRequest {
            token: 1,
            method: "GET".to_owned(),
            url: url.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn ftp_is_rejected_before_a_connection() {
        match perform_http(&request("ftp://example.test")) {
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
        let result = perform_http(&request(&format!("http://127.0.0.1:{port}/")));
        match result {
            Ok(response) => assert_eq!(response.body, b"hello"),
            Err(error) => panic!("{}", error.message),
        }
        let _ = server.join();
    }
}
