//! WebSocket work for `uifw-net`. HTTP is [`crate::http::OpenWireClient`].

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use session::{
    HttpClient, HttpError, HttpRequest, IoJob, PluginResult, PluginValue, module_not_linked,
};

use crate::command::Command;
use crate::platform::PlatformBridge;
use crate::queue::Inbox;

pub(crate) fn spawn(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    bridge: Arc<dyn PlatformBridge>,
    http: Option<Arc<dyn HttpClient>>,
) -> Option<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("uifw-net".to_string())
        .spawn(move || serve(jobs, inbox, bridge, http))
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

fn serve(
    jobs: Receiver<IoJob>,
    inbox: Arc<Inbox>,
    bridge: Arc<dyn PlatformBridge>,
    http: Option<Arc<dyn HttpClient>>,
) {
    let mut sockets: HashMap<u64, Sender<SocketCmd>> = HashMap::new();
    while let Ok(job) = jobs.recv() {
        match job {
            IoJob::Http(request) => {
                if request.cancel.load(Ordering::Acquire) {
                    continue;
                }
                let token = request.token;
                let result = match http.as_ref() {
                    Some(client) => client.perform(&request),
                    None => Err(HttpError::new("network is not linked")),
                };
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
    let root = || {
        bridge
            .document_dir()
            .map(|dir| std::path::PathBuf::from(dir))
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
        ("FileSystem", "documentDirectory") => match bridge.document_dir() {
            Ok(dir) => PluginResult::Done(PluginValue::Str(dir)),
            Err(message) => PluginResult::Failed { message },
        },
        ("FileSystem", "read") => {
            match root().and_then(|dir| crate::files::read_document(&dir, text)) {
                Ok(bytes) => PluginResult::Done(PluginValue::Bytes(bytes)),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("FileSystem", "write") => {
            let bytes = match args.get(1) {
                Some(PluginValue::Bytes(bytes)) => bytes.clone(),
                Some(PluginValue::Str(value)) => value.as_bytes().to_vec(),
                _ => Vec::new(),
            };
            match root().and_then(|dir| crate::files::write_document(&dir, text, &bytes)) {
                Ok(()) => PluginResult::Done(PluginValue::Null),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("FileSystem", "delete") => {
            match root().and_then(|dir| crate::files::delete_document(&dir, text)) {
                Ok(()) => PluginResult::Done(PluginValue::Null),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("FileSystem", "pick") => match bridge.pick_document() {
            Ok(Some(name)) => PluginResult::Done(PluginValue::Map(vec![(
                "name".to_owned(),
                PluginValue::Str(name),
            )])),
            Ok(None) => PluginResult::Done(PluginValue::Null),
            Err(message) => PluginResult::Failed { message },
        },
        ("AsyncStorage", "multiGet") => {
            match root().and_then(|dir| crate::files::store_get(&dir, &string_list(args.first()))) {
                Ok(rows) => PluginResult::Done(PluginValue::List(
                    rows.into_iter()
                        .map(|(key, value)| {
                            PluginValue::List(vec![
                                PluginValue::Str(key),
                                value.map(PluginValue::Str).unwrap_or(PluginValue::Null),
                            ])
                        })
                        .collect(),
                )),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("AsyncStorage", "multiSet") => {
            match root().and_then(|dir| crate::files::store_set(&dir, &string_pairs(args.first())))
            {
                Ok(()) => PluginResult::Done(PluginValue::Null),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("AsyncStorage", "multiRemove") => {
            match root()
                .and_then(|dir| crate::files::store_remove(&dir, &string_list(args.first())))
            {
                Ok(()) => PluginResult::Done(PluginValue::Null),
                Err(message) => PluginResult::Failed { message },
            }
        }
        ("AsyncStorage", "clear") => match root().and_then(|dir| crate::files::store_clear(&dir)) {
            Ok(()) => PluginResult::Done(PluginValue::Null),
            Err(message) => PluginResult::Failed { message },
        },
        ("Permissions", "checkPermissionStatus") => {
            PluginResult::Done(PluginValue::Str(bridge.permission_status(text)))
        }
        ("Permissions", "requestPermissions") => {
            let rows = bridge
                .request_permissions(&string_list(args.first()))
                .into_iter()
                .map(|(name, status)| (name, PluginValue::Str(status)))
                .collect();
            PluginResult::Done(PluginValue::Map(rows))
        }
        ("Permissions", "shouldShowRequestPermissionRationale") => {
            PluginResult::Done(PluginValue::Bool(bridge.permission_rationale(text)))
        }
        ("Permissions", "checkServiceStatus") => {
            PluginResult::Done(PluginValue::Str(bridge.service_status(text)))
        }
        ("Permissions", "openAppSettings") => {
            PluginResult::Done(PluginValue::Bool(bridge.open_app_settings()))
        }
        _ => PluginResult::Failed {
            message: module_not_linked(name, Some(method)),
        },
    }
}

fn string_list(value: Option<&PluginValue>) -> Vec<String> {
    let Some(PluginValue::List(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            PluginValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn string_pairs(value: Option<&PluginValue>) -> Vec<(String, String)> {
    let Some(PluginValue::List(rows)) = value else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let PluginValue::List(pair) = row else {
                return None;
            };
            match (pair.first(), pair.get(1)) {
                (Some(PluginValue::Str(key)), Some(PluginValue::Str(value))) => {
                    Some((key.clone(), value.clone()))
                }
                _ => None,
            }
        })
        .collect()
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
}
