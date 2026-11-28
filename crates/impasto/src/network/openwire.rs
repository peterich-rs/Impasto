use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use http::{Request, Response, Uri, Version};
use openwire::{
    CallContext, ConnectionId, EventListener, EventListenerFactory, NoopEventListener, ProxyEvent,
    RequestBody, ResponseBody, TlsHandshake, WireError,
};

use super::{ConnectionMeta, HopLog, NetworkTiming, RedirectHop};

/// Builds isolated listeners from the request's extensions.
pub struct OpenWireObserver;

/// One request hop. A drop also publishes timings when reception is canceled.
pub struct OpenWireHop(Arc<Hop>);

#[derive(Default)]
struct Marks {
    queue: Pair,
    proxy: Pair,
    dns: Pair,
    tcp: Pair,
    tls: Pair,
    sent: Pair,
    headers: Pair,
    download: Pair,
    ended: Option<Instant>,
    connection: ConnectionMeta,
}

#[derive(Default)]
struct Pair(Option<Instant>, Option<Instant>);

impl Pair {
    fn start(&mut self) {
        self.0 = Some(Instant::now());
    }
    fn end(&mut self) {
        self.1 = Some(Instant::now());
    }
    fn elapsed(&self) -> Option<f64> {
        elapsed(self.0, self.1)
    }
}

fn elapsed(start: Option<Instant>, end: Option<Instant>) -> Option<f64> {
    Some(end?.checked_duration_since(start?)?.as_secs_f64() * 1000.0)
}

struct Hop {
    marks: Mutex<Marks>,
    start: Instant,
    method: String,
    url: String,
    log: HopLog,
    response: Mutex<(u16, Option<String>)>,
}

impl OpenWireHop {
    /// Associates a call with its observation without modifying shared client state.
    pub fn install(request: &mut Request<RequestBody>, log: HopLog) -> Self {
        let hop = Arc::new(Hop {
            marks: Mutex::new(Marks::default()),
            start: Instant::now(),
            method: request.method().to_string(),
            url: request.uri().to_string(),
            log,
            response: Mutex::new((0, None)),
        });
        request.extensions_mut().insert(Arc::clone(&hop));
        Self(hop)
    }

    /// Sets the response status and redirect location recorded for this hop.
    pub fn response(&self, status: u16, location: Option<String>) {
        *self
            .0
            .response
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = (status, location);
    }
}

impl Drop for OpenWireHop {
    fn drop(&mut self) {
        let marks = self
            .0
            .marks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (status, location) = self
            .0
            .response
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let reused = marks.connection.reused;
        self.0.log.add(RedirectHop {
            index: 0,
            method: self.0.method.clone(),
            url: self.0.url.clone(),
            status,
            location,
            timing: NetworkTiming {
                queue_ms: marks.queue.elapsed(),
                proxy_ms: marks.proxy.elapsed(),
                dns_ms: if reused { None } else { marks.dns.elapsed() },
                tcp_ms: if reused {
                    None
                } else {
                    elapsed(
                        marks.tcp.0,
                        marks.tls.0.into_iter().chain(marks.tcp.1).min(),
                    )
                },
                tls_ms: if reused { None } else { marks.tls.elapsed() },
                request_sent_ms: marks.sent.elapsed(),
                waiting_ttfb_ms: elapsed(marks.sent.1, marks.headers.0),
                download_ms: elapsed(
                    marks.download.0.or(marks.headers.1),
                    marks.download.1.or(marks.ended),
                ),
                total_ms: self.0.start.elapsed().as_secs_f64() * 1000.0,
            },
            connection: marks.connection.clone(),
        });
    }
}

impl EventListenerFactory for OpenWireObserver {
    fn create(&self, request: &Request<RequestBody>) -> Arc<dyn EventListener> {
        request.extensions().get::<Arc<Hop>>().cloned().map_or_else(
            || Arc::new(NoopEventListener) as Arc<dyn EventListener>,
            |hop| hop as Arc<dyn EventListener>,
        )
    }
}

impl Hop {
    fn update(&self, f: impl FnOnce(&mut Marks)) {
        f(&mut self.marks.lock().unwrap_or_else(|error| error.into_inner()));
    }
}

impl EventListener for Hop {
    fn dispatcher_queue_start(&self, _: &CallContext) {
        self.update(|m| m.queue.start());
    }
    fn dispatcher_queue_end(&self, _: &CallContext) {
        self.update(|m| m.queue.end());
    }
    fn proxy_select_start(&self, _: &CallContext, _: &Uri) {
        self.update(|m| m.proxy.start());
    }
    fn proxy_select_end(&self, _: &CallContext, _: &Uri, _: &[ProxyEvent]) {
        self.update(|m| m.proxy.end());
    }
    fn dns_start(&self, _: &CallContext, _: &str, _: u16) {
        self.update(|m| m.dns.start());
    }
    fn dns_end(&self, _: &CallContext, _: &str, addresses: &[SocketAddr]) {
        self.update(|m| {
            m.dns.end();
            m.connection.resolved_ips = addresses
                .iter()
                .take(32)
                .map(|address| address.ip().to_string())
                .collect();
        });
    }
    fn connect_start(&self, _: &CallContext, _: SocketAddr) {
        self.update(|m| m.tcp.start());
    }
    fn connect_end(&self, _: &CallContext, _: ConnectionId, address: SocketAddr) {
        self.update(|m| {
            m.tcp.end();
            m.connection.remote_addr = Some(address.to_string());
        });
    }
    fn tls_start(&self, _: &CallContext, _: &str) {
        self.update(|m| m.tls.start());
    }
    fn tls_end(&self, _: &CallContext, _: &str) {
        self.update(|m| m.tls.end());
    }
    fn tls_handshake(&self, _: &CallContext, _: &str, handshake: &TlsHandshake) {
        self.update(|m| {
            m.connection.tls_version = handshake.protocol_version.clone();
            m.connection.tls_cipher = handshake.cipher_suite.clone();
            m.connection.alpn = handshake
                .alpn
                .as_ref()
                .map(|alpn| String::from_utf8_lossy(alpn).into_owned());
        });
    }
    fn connection_acquired(&self, _: &CallContext, _: ConnectionId, reused: bool) {
        self.update(|m| m.connection.reused = reused);
    }
    fn request_headers_start(&self, _: &CallContext) {
        self.update(|m| m.sent.start());
    }
    fn request_headers_end(&self, _: &CallContext) {
        self.update(|m| m.sent.end());
    }
    fn request_body_end(&self, _: &CallContext, _: u64) {
        self.update(|m| m.sent.end());
    }
    fn response_headers_start(&self, _: &CallContext) {
        self.update(|m| m.headers.start());
    }
    fn response_headers_end(&self, _: &CallContext, response: &Response<ResponseBody>) {
        self.update(|m| {
            m.headers.end();
            m.connection.protocol = Some(
                match response.version() {
                    Version::HTTP_09 => "http/0.9",
                    Version::HTTP_10 => "http/1.0",
                    Version::HTTP_11 => "http/1.1",
                    Version::HTTP_2 => "h2",
                    Version::HTTP_3 => "h3",
                    _ => "unknown",
                }
                .to_owned(),
            );
        });
    }
    fn response_body_start(&self, _: &CallContext) {
        self.update(|m| m.download.start());
    }
    fn response_body_end(&self, _: &CallContext, _: u64) {
        self.update(|m| m.download.end());
    }
    fn call_end(&self, _: &CallContext) {
        self.update(|m| m.ended = Some(Instant::now()));
    }
    fn call_failed(&self, _: &CallContext, _: &WireError) {
        self.update(|m| m.ended = Some(Instant::now()));
    }
}
