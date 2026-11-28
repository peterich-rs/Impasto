//! Native async WebSocket runner and platform transport contract.

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::time::Duration;

use crate::trace;
use crate::{NativeWebSocketEvent, SocketData, SocketError};
use bytes::Bytes;
use futures_util::future::{Either, select};
use futures_util::{SinkExt as _, StreamExt as _};
use http::Uri;
use impasto::{
    Bytes as JsBytes, CancelToken, DiagnosticsSink, ImpastoError, ImpastoType, TypedSink,
};
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, WebSocketConfig};

pub const MAX_SOCKETS: usize = 8;
pub const MAX_OUTBOUND_BUFFERED: usize = 256 << 10;

const CMD_CHANNEL_CAP: usize = 256;
const BLOCKING_SEND_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub const READY_CONNECTING: u8 = 0;
pub const READY_OPEN: u8 = 1;
pub const READY_CLOSING: u8 = 2;
pub const READY_CLOSED: u8 = 3;

#[derive(Debug, Clone)]
pub enum WebSocketCommand {
    Send { text: bool, data: Vec<u8> },
    Close { code: Option<u16>, reason: String },
}

#[derive(Debug, Clone)]
pub enum WebSocketEvent {
    Open {
        protocol: String,
    },
    Message {
        text: bool,
        data: Vec<u8>,
    },
    Close {
        code: u16,
        reason: String,
        clean: bool,
    },
    Error(String),
}

#[derive(Default, ImpastoType)]
pub struct WebSocketSettings {
    pub headers: Option<Vec<(String, String)>>,
    pub heartbeat: Option<Heartbeat>,
}

#[derive(Clone, Copy, ImpastoType)]
pub struct Heartbeat {
    interval_ms: u32,
    timeout_ms: Option<u32>,
}

impl Heartbeat {
    pub fn interval(self) -> Duration {
        Duration::from_millis(u64::from(self.interval_ms))
    }
    pub fn timeout(self) -> Duration {
        Duration::from_millis(u64::from(self.timeout_ms.unwrap_or(self.interval_ms)))
    }
    fn valid(self) -> bool {
        !self.interval().is_zero() && !self.timeout().is_zero()
    }
}

fn handshake_headers(headers: &[(String, String)]) -> Result<http::HeaderMap, String> {
    let mut output = http::HeaderMap::new();
    for (name, value) in headers {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "invalid WebSocket handshake header".to_owned())?;
        if name.as_str().starts_with("sec-websocket-")
            || matches!(
                name.as_str(),
                "upgrade" | "connection" | "host" | "content-length" | "transfer-encoding"
            )
        {
            return Err("reserved WebSocket handshake header".to_owned());
        }
        let value = http::HeaderValue::from_str(value)
            .map_err(|_| "invalid WebSocket handshake header value".to_owned())?;
        output.append(name, value);
    }
    Ok(output)
}

type SentObserver<'a> = dyn FnMut(bool, &[u8]) + 'a;

#[cfg_attr(
    not(target_os = "ios"),
    expect(
        dead_code,
        reason = "The native transport consuming these callbacks is iOS-only"
    )
)]
pub(crate) struct TransportEvents<'a> {
    pub emit: &'a mut dyn FnMut(WebSocketEvent) -> bool,
    pub sent: Option<&'a mut SentObserver<'a>>,
}

pub(crate) trait WebSocketTransport: Send + Sync {
    fn run(
        &self,
        url: &str,
        protocols: &[String],
        settings: &WebSocketSettings,
        commands: std_mpsc::Receiver<WebSocketCommand>,
        active: &dyn Fn() -> bool,
        events: TransportEvents<'_>,
    ) -> Result<(), String>;
}

pub struct SocketSlotTracker(Arc<AtomicUsize>);

impl Default for SocketSlotTracker {
    fn default() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
}

impl SocketSlotTracker {
    pub(crate) fn acquire(&self) -> Result<SlotTicket, String> {
        let mut current = self.0.load(Ordering::Acquire);
        loop {
            if current >= MAX_SOCKETS {
                return Err(format!("too many open WebSockets (limit {MAX_SOCKETS})"));
            }
            match self.0.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(SlotTicket(Arc::clone(&self.0))),
                Err(actual) => current = actual,
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn active_count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct SlotTicket(Arc<AtomicUsize>);

impl Drop for SlotTicket {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct QueuedCommand {
    len: usize,
    command: WebSocketCommand,
}

struct PendingSocketRun {
    url: String,
    protocols: Vec<String>,
    settings: WebSocketSettings,
    rx: mpsc::Receiver<QueuedCommand>,
    transport: Option<Arc<dyn WebSocketTransport>>,
    devtools: Option<Arc<dyn DiagnosticsSink>>,
}

#[derive(Debug, Clone)]
struct CloseRequest {
    code: Option<u16>,
    reason: String,
}

type PendingClose = Arc<Mutex<Option<CloseRequest>>>;

pub struct SocketSession {
    id: u64,
    tx: mpsc::Sender<QueuedCommand>,
    buffered_bytes: Arc<AtomicUsize>,
    ready_state: Arc<AtomicU8>,
    protocol: Arc<Mutex<String>>,
    cancel: CancelToken,
    close_notify: Arc<Notify>,
    pending_close: PendingClose,
    pending_run: Mutex<Option<(SlotTicket, PendingSocketRun)>>,
}

impl SocketSession {
    pub fn open(
        url: String,
        protocols: Vec<String>,
        settings: WebSocketSettings,
        tracker: &SocketSlotTracker,
        transport: Option<Arc<dyn WebSocketTransport>>,
        devtools: Option<Arc<dyn DiagnosticsSink>>,
    ) -> Result<Arc<Self>, String> {
        validate_ws_url(&url)?;
        validate_subprotocols(&protocols)?;
        handshake_headers(settings.headers.as_deref().unwrap_or_default())?;
        if settings
            .heartbeat
            .is_some_and(|heartbeat| !heartbeat.valid())
        {
            return Err("invalid WebSocket heartbeat interval".to_owned());
        }
        let ticket = tracker.acquire()?;
        let (tx, rx) = mpsc::channel(CMD_CHANNEL_CAP);
        let id = impasto::network::next_token();

        Ok(Arc::new(Self {
            id,
            tx,
            buffered_bytes: Arc::new(AtomicUsize::new(0)),
            ready_state: Arc::new(AtomicU8::new(READY_CONNECTING)),
            protocol: Arc::new(Mutex::new(String::new())),
            cancel: CancelToken::new(),
            close_notify: Arc::new(Notify::new()),
            pending_close: Arc::new(Mutex::new(None)),
            pending_run: Mutex::new(Some((
                ticket,
                PendingSocketRun {
                    url,
                    protocols,
                    settings,
                    rx,
                    transport,
                    devtools,
                },
            ))),
        }))
    }

    /// ID assigned to the session for its native event stream.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn ready_state(&self) -> u8 {
        self.ready_state.load(Ordering::Acquire)
    }

    pub fn buffered_amount(&self) -> usize {
        self.buffered_bytes.load(Ordering::Acquire)
    }

    pub fn protocol(&self) -> String {
        self.protocol
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn send_text(&self, text: String) -> Result<usize, String> {
        self.enqueue_data(true, text.into_bytes())
    }

    pub fn send_binary(&self, data: Vec<u8>) -> Result<usize, String> {
        self.enqueue_data(false, data)
    }

    fn enqueue_data(&self, text: bool, data: Vec<u8>) -> Result<usize, String> {
        let state = self.ready_state();
        if state == READY_CONNECTING {
            return Err("WebSocket is still connecting".to_owned());
        }
        if state != READY_OPEN {
            return Err("WebSocket is not open".to_owned());
        }

        let len = data.len();
        let mut current = self.buffered_bytes.load(Ordering::Acquire);
        let updated = loop {
            let next = current.saturating_add(len);
            if next > MAX_OUTBOUND_BUFFERED {
                return Err("outbound WebSocket buffer is full".to_owned());
            }
            match self.buffered_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break next,
                Err(actual) => current = actual,
            }
        };

        let queued = QueuedCommand {
            len,
            command: WebSocketCommand::Send { text, data },
        };
        if let Err(err) = self.tx.try_send(queued) {
            self.buffered_bytes.fetch_sub(len, Ordering::AcqRel);
            return Err(match err {
                mpsc::error::TrySendError::Full(_) => {
                    "outbound WebSocket buffer is full".to_owned()
                }
                mpsc::error::TrySendError::Closed(_) => "WebSocket is closed".to_owned(),
            });
        }
        Ok(updated)
    }

    pub fn close(&self, code: Option<u16>, reason: String) -> Result<(), String> {
        validate_close_args(code, &reason)?;
        let prev = self
            .ready_state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state == READY_CONNECTING || state == READY_OPEN).then_some(READY_CLOSING)
            });
        if prev.is_err() {
            return Ok(());
        }

        *self.pending_close.lock().unwrap_or_else(|p| p.into_inner()) = Some(CloseRequest {
            code,
            reason: reason.clone(),
        });
        self.close_notify.notify_waiters();

        if prev == Ok(READY_OPEN) {
            let _ = self.tx.try_send(QueuedCommand {
                len: 0,
                command: WebSocketCommand::Close { code, reason },
            });
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.cancel.cancel();
        self.close_notify.notify_waiters();
        let _ = self
            .pending_run
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }

    pub async fn start_stream(&self, sink: TypedSink<NativeWebSocketEvent>) -> Result<(), String> {
        let (ticket, run) = self
            .pending_run
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or("WebSocket event stream was already started or closed")?;
        let handle = tokio::runtime::Handle::try_current().map_err(|error| error.to_string())?;
        let shared = SocketShared {
            id: self.id,
            buffered_bytes: Arc::clone(&self.buffered_bytes),
            ready_state: Arc::clone(&self.ready_state),
            protocol: Arc::clone(&self.protocol),
            cancel: self.cancel.clone(),
            sink_cancel: sink.cancellation(),
            close_notify: Arc::clone(&self.close_notify),
            pending_close: Arc::clone(&self.pending_close),
            ticket: Mutex::new(Some(ticket)),
        };
        run_socket(run, shared, sink, handle).await;
        Ok(())
    }
}

struct SocketShared {
    id: u64,
    buffered_bytes: Arc<AtomicUsize>,
    ready_state: Arc<AtomicU8>,
    protocol: Arc<Mutex<String>>,
    cancel: CancelToken,
    sink_cancel: CancelToken,
    close_notify: Arc<Notify>,
    pending_close: PendingClose,
    ticket: Mutex<Option<SlotTicket>>,
}

impl SocketShared {
    fn mark_closed(&self) {
        // JavaScript can open a replacement as soon as it sees the close
        // event. Release the slot before publishing that event.
        let ticket = self.ticket.lock().unwrap_or_else(|p| p.into_inner()).take();
        drop(ticket);
        self.ready_state.store(READY_CLOSED, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled() || self.sink_cancel.is_cancelled()
    }

    async fn cancelled(&self) {
        let a = std::pin::pin!(self.cancel.cancelled());
        let b = std::pin::pin!(self.sink_cancel.cancelled());
        let _ = select(a, b).await;
    }

    fn record_event(&self, sink: Option<&dyn DiagnosticsSink>, url: &str, event: &WebSocketEvent) {
        if let Some(sink) = sink {
            trace::record(sink, self.id, url, event);
        }
    }
}

impl Drop for SocketShared {
    fn drop(&mut self) {
        self.mark_closed();
    }
}

async fn run_socket(
    mut run: PendingSocketRun,
    shared: SocketShared,
    mut sink: TypedSink<NativeWebSocketEvent>,
    handle: tokio::runtime::Handle,
) {
    let _trace = run.devtools.as_ref().map(|sink| {
        trace::SocketTrace::start(Arc::clone(sink), shared.id, &run.url, shared.cancel.clone())
    });
    if let Some(platform) = run.transport.take() {
        run_blocking_transport(platform, run, shared, sink, handle).await;
        return;
    }
    let PendingSocketRun {
        url,
        protocols,
        settings,
        rx,
        devtools,
        ..
    } = run;
    run_native_async(url, protocols, settings, rx, &shared, &mut sink, devtools).await;
    shared.mark_closed();
}

async fn run_native_async(
    url: String,
    protocols: Vec<String>,
    settings: WebSocketSettings,
    mut rx: mpsc::Receiver<QueuedCommand>,
    shared: &SocketShared,
    sink: &mut TypedSink<NativeWebSocketEvent>,
    devtools: Option<Arc<dyn DiagnosticsSink>>,
) {
    let request = match build_ws_request(&url, &protocols).and_then(|mut request| {
        request.headers_mut().extend(handshake_headers(
            settings.headers.as_deref().unwrap_or_default(),
        )?);
        Ok(request)
    }) {
        Ok(req) => req,
        Err(message) => {
            emit_error_and_close(sink, shared, devtools.as_deref(), &url, message, 1006).await;
            return;
        }
    };

    let config = WebSocketConfig::default();
    let connect_fut = std::pin::pin!(connect_async_with_config(request, Some(config), false));
    let abort_fut = std::pin::pin!(shared.cancelled());
    let close_fut = std::pin::pin!(shared.close_notify.notified());

    let (ws_stream, handshake_response) =
        match select(select(abort_fut, close_fut), connect_fut).await {
            Either::Left((Either::Left(((), _)), _)) => {
                return;
            }
            Either::Left((Either::Right(((), _)), _)) => {
                let CloseRequest { code, reason } = shared
                    .pending_close
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                    .unwrap_or(CloseRequest {
                        code: Some(1000),
                        reason: String::new(),
                    });
                let ev = WebSocketEvent::Close {
                    code: code.unwrap_or(1000),
                    reason,
                    clean: false,
                };
                shared.mark_closed();
                shared.record_event(devtools.as_deref(), &url, &ev);
                let _ = send_event_async(sink, ev).await;
                return;
            }
            Either::Right((Ok(pair), _)) => pair,
            Either::Right((Err(err), _)) => {
                emit_error_and_close(
                    sink,
                    shared,
                    devtools.as_deref(),
                    &url,
                    err.to_string(),
                    1006,
                )
                .await;
                return;
            }
        };

    let negotiated = handshake_response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_default();
    *shared.protocol.lock().unwrap_or_else(|p| p.into_inner()) = negotiated.clone();

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let pending_close = shared
        .pending_close
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();

    if pending_close.is_none() {
        shared.ready_state.store(READY_OPEN, Ordering::Release);
        let open_ev = WebSocketEvent::Open {
            protocol: negotiated,
        };
        shared.record_event(devtools.as_deref(), &url, &open_ev);
        if !send_event_async(sink, open_ev).await {
            let _ = ws_tx.close().await;
            return;
        }
    }

    let mut close_sent = false;
    if let Some(CloseRequest { code, reason }) = pending_close {
        close_sent = true;
        shared.ready_state.store(READY_CLOSING, Ordering::Release);
        let _ = ws_tx.send(make_close_message(code, reason)).await;
    }

    let max_frame_bytes = sink.max_piece_bytes().saturating_sub(128);
    let mut close_deadline: Option<tokio::time::Instant> =
        close_sent.then(|| tokio::time::Instant::now() + CLOSE_HANDSHAKE_TIMEOUT);

    let heartbeat = settings.heartbeat;
    let mut next_ping =
        heartbeat.map(|heartbeat| tokio::time::Instant::now() + heartbeat.interval());
    let mut pong_deadline = None;
    loop {
        if shared.is_cancelled() {
            let _ = ws_tx.close().await;
            break;
        }

        enum Step {
            Cancel,
            Incoming(Option<Result<Message, TungsteniteError>>),
            Command(Option<QueuedCommand>),
            CloseTimeout,
            Ping,
            PongTimeout,
        }

        let step = tokio::select! {
            () = shared.cancelled() => Step::Cancel,
            () = until(close_deadline) => Step::CloseTimeout,
            () = until(next_ping.filter(|_| !close_sent && pong_deadline.is_none())) => Step::Ping,
            () = until(pong_deadline.filter(|_| !close_sent)) => Step::PongTimeout,
            message = ws_rx.next() => Step::Incoming(message),
            command = rx.recv(), if !close_sent => Step::Command(command),
        };

        match step {
            Step::Ping => {
                if let Some(heartbeat) = heartbeat {
                    if ws_tx.send(Message::Ping(Bytes::new())).await.is_err() {
                        emit_error_and_close(
                            sink,
                            shared,
                            devtools.as_deref(),
                            &url,
                            "WebSocket ping failed".into(),
                            1006,
                        )
                        .await;
                        break;
                    }
                    pong_deadline = Some(tokio::time::Instant::now() + heartbeat.timeout());
                }
            }
            Step::PongTimeout => {
                let _ = ws_tx
                    .send(make_close_message(Some(1000), String::new()))
                    .await;
                emit_error_and_close(
                    sink,
                    shared,
                    devtools.as_deref(),
                    &url,
                    "WebSocket heartbeat timed out".into(),
                    1006,
                )
                .await;
                break;
            }
            Step::Cancel => {
                let _ = ws_tx.close().await;
                break;
            }
            Step::CloseTimeout => {
                let ev = WebSocketEvent::Close {
                    code: 1006,
                    reason: "close handshake timed out".to_owned(),
                    clean: false,
                };
                shared.mark_closed();
                shared.record_event(devtools.as_deref(), &url, &ev);
                let _ = send_event_async(sink, ev).await;
                break;
            }
            Step::Command(None) => {
                // Handle dropped: initiate clean close if still open.
                if !close_sent {
                    close_sent = true;
                    shared.ready_state.store(READY_CLOSING, Ordering::Release);
                    let _ = ws_tx
                        .send(make_close_message(Some(1000), String::new()))
                        .await;
                    close_deadline = Some(tokio::time::Instant::now() + CLOSE_HANDSHAKE_TIMEOUT);
                }
            }
            Step::Command(Some(QueuedCommand { len, command })) => {
                let send_res = match command {
                    WebSocketCommand::Send { text, data } => {
                        let msg = if text {
                            match String::from_utf8(data) {
                                Ok(text) => Ok(Message::Text(text.into())),
                                Err(err) => Err(err.to_string()),
                            }
                        } else {
                            Ok(Message::Binary(Bytes::from(data)))
                        };
                        match msg {
                            Ok(msg) => {
                                let observed = devtools.is_some().then(|| msg.clone());
                                let result =
                                    ws_tx.send(msg).await.map_err(|error| error.to_string());
                                if result.is_ok()
                                    && let (Some(sink), Some(message)) =
                                        (devtools.as_deref(), observed)
                                {
                                    trace::outbound(
                                        sink,
                                        shared.id,
                                        &url,
                                        text,
                                        &message.into_data(),
                                    );
                                }
                                result
                            }
                            Err(err) => Err(err),
                        }
                    }
                    WebSocketCommand::Close { code, reason } => {
                        close_sent = true;
                        shared.ready_state.store(READY_CLOSING, Ordering::Release);
                        close_deadline =
                            Some(tokio::time::Instant::now() + CLOSE_HANDSHAKE_TIMEOUT);
                        ws_tx
                            .send(make_close_message(code, reason))
                            .await
                            .map_err(|e| e.to_string())
                    }
                };
                if len > 0 {
                    shared.buffered_bytes.fetch_sub(len, Ordering::AcqRel);
                }
                if let Err(err) = send_res {
                    emit_error_and_close(sink, shared, devtools.as_deref(), &url, err, 1006).await;
                    break;
                }
            }
            Step::Incoming(None) => {
                let ev = WebSocketEvent::Close {
                    code: 1006,
                    reason: String::new(),
                    clean: false,
                };
                shared.mark_closed();
                shared.record_event(devtools.as_deref(), &url, &ev);
                let _ = send_event_async(sink, ev).await;
                break;
            }
            Step::Incoming(Some(Err(err))) => {
                emit_error_and_close(
                    sink,
                    shared,
                    devtools.as_deref(),
                    &url,
                    err.to_string(),
                    1006,
                )
                .await;
                break;
            }
            Step::Incoming(Some(Ok(message))) => match message {
                Message::Text(text) => {
                    let data = text.as_bytes().to_vec();
                    if data.len() > max_frame_bytes {
                        let _ = ws_tx
                            .send(make_close_message(Some(1009), "message too big".into()))
                            .await;
                        emit_error_and_close(
                            sink,
                            shared,
                            devtools.as_deref(),
                            &url,
                            "incoming WebSocket message exceeds frame limit".to_owned(),
                            1009,
                        )
                        .await;
                        break;
                    }
                    let ev = WebSocketEvent::Message { text: true, data };
                    shared.record_event(devtools.as_deref(), &url, &ev);
                    if !send_event_async(sink, ev).await {
                        let _ = ws_tx.close().await;
                        break;
                    }
                }
                Message::Binary(data) => {
                    if data.len() > max_frame_bytes {
                        let _ = ws_tx
                            .send(make_close_message(Some(1009), "message too big".into()))
                            .await;
                        emit_error_and_close(
                            sink,
                            shared,
                            devtools.as_deref(),
                            &url,
                            "incoming WebSocket message exceeds frame limit".to_owned(),
                            1009,
                        )
                        .await;
                        break;
                    }
                    let ev = WebSocketEvent::Message {
                        text: false,
                        data: data.to_vec(),
                    };
                    shared.record_event(devtools.as_deref(), &url, &ev);
                    if !send_event_async(sink, ev).await {
                        let _ = ws_tx.close().await;
                        break;
                    }
                }
                Message::Close(frame) => {
                    let (code, reason) = match frame {
                        Some(frame) => (u16::from(frame.code), frame.reason.to_string()),
                        None => (1005, String::new()),
                    };
                    if !close_sent {
                        let _ = ws_tx
                            .send(make_close_message(
                                (code != 1005).then_some(code),
                                reason.clone(),
                            ))
                            .await;
                    }
                    shared.mark_closed();
                    let ev = WebSocketEvent::Close {
                        code,
                        reason,
                        clean: true,
                    };
                    shared.record_event(devtools.as_deref(), &url, &ev);
                    let _ = send_event_async(sink, ev).await;
                    break;
                }
                Message::Pong(payload) if payload.is_empty() && pong_deadline.take().is_some() => {
                    next_ping = heartbeat
                        .map(|heartbeat| tokio::time::Instant::now() + heartbeat.interval());
                }
                Message::Ping(_) => {
                    let _ = ws_tx.flush().await;
                }
                Message::Pong(_) | Message::Frame(_) => {}
            },
        }
    }
}

async fn until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn run_blocking_transport(
    transport: Arc<dyn WebSocketTransport>,
    run: PendingSocketRun,
    shared: SocketShared,
    mut sink: TypedSink<NativeWebSocketEvent>,
    handle: tokio::runtime::Handle,
) {
    let PendingSocketRun {
        url,
        protocols,
        settings,
        mut rx,
        devtools,
        ..
    } = run;
    let (std_tx, std_rx) = std_mpsc::sync_channel::<WebSocketCommand>(CMD_CHANNEL_CAP);
    let forward_buffered = Arc::clone(&shared.buffered_bytes);
    let forward_tx = std_tx.clone();
    let forward_task = handle.spawn(async move {
        while let Some(QueuedCommand { len, command }) = rx.recv().await {
            if forward_tx.send(command).is_err() {
                if len > 0 {
                    forward_buffered.fetch_sub(len, Ordering::AcqRel);
                }
                break;
            }
            if len > 0 {
                forward_buffered.fetch_sub(len, Ordering::AcqRel);
            }
        }
    });

    let active_cancel = shared.cancel.clone();
    let active_sink_cancel = shared.sink_cancel.clone();
    let emit_cancel = shared.cancel.clone();
    let emit_sink_cancel = shared.sink_cancel.clone();

    let join = handle.spawn_blocking(move || {
        let mut record_sent = |text, bytes: &[u8]| {
            if let Some(diagnostics) = devtools.as_deref() {
                trace::outbound(diagnostics, shared.id, &url, text, bytes);
            }
        };
        let observe = devtools
            .is_some()
            .then_some(&mut record_sent as &mut SentObserver<'_>);
        let outcome = transport.run(
            &url,
            &protocols,
            &settings,
            std_rx,
            &|| !active_cancel.is_cancelled() && !active_sink_cancel.is_cancelled(),
            TransportEvents {
                emit: &mut |event| {
                    if emit_cancel.is_cancelled() || emit_sink_cancel.is_cancelled() {
                        return false;
                    }
                    match &event {
                        WebSocketEvent::Open { protocol } => {
                            *shared.protocol.lock().unwrap_or_else(|p| p.into_inner()) =
                                protocol.clone();
                            shared.ready_state.store(READY_OPEN, Ordering::Release);
                        }
                        WebSocketEvent::Close { .. } => {
                            shared.mark_closed();
                        }
                        WebSocketEvent::Message { .. } | WebSocketEvent::Error(_) => {}
                    }
                    shared.record_event(devtools.as_deref(), &url, &event);
                    sink.send_blocking(encode_event(event), BLOCKING_SEND_TIMEOUT)
                        .is_ok()
                },
                sent: observe,
            },
        );
        shared.mark_closed();
        if let Err(message) = outcome
            && !emit_cancel.is_cancelled()
            && !emit_sink_cancel.is_cancelled()
        {
            let event = WebSocketEvent::Error(message);
            shared.record_event(devtools.as_deref(), &url, &event);
            let _ = sink.send_blocking(encode_event(event), BLOCKING_SEND_TIMEOUT);
        }
    });

    let _ = join.await;
    forward_task.abort();
}

async fn emit_error_and_close(
    sink: &mut TypedSink<NativeWebSocketEvent>,
    shared: &SocketShared,
    devtools: Option<&dyn DiagnosticsSink>,
    url: &str,
    message: String,
    close_code: u16,
) {
    let err_ev = WebSocketEvent::Error(message);
    shared.mark_closed();
    shared.record_event(devtools, url, &err_ev);
    if send_event_async(sink, err_ev).await {
        let close_ev = WebSocketEvent::Close {
            code: close_code,
            reason: String::new(),
            clean: false,
        };
        shared.record_event(devtools, url, &close_ev);
        let _ = send_event_async(sink, close_ev).await;
    }
}

async fn send_event_async(
    sink: &mut TypedSink<NativeWebSocketEvent>,
    event: WebSocketEvent,
) -> bool {
    sink.send(encode_event(event)).await.is_ok()
}

fn encode_event(event: WebSocketEvent) -> NativeWebSocketEvent {
    match event {
        WebSocketEvent::Open { protocol } => NativeWebSocketEvent::Open { protocol },
        WebSocketEvent::Message { text, data } => {
            let data = if text {
                match String::from_utf8(data) {
                    Ok(text) => SocketData::Text(text),
                    Err(error) => SocketData::Bytes(JsBytes(error.into_bytes())),
                }
            } else {
                SocketData::Bytes(JsBytes(data))
            };
            NativeWebSocketEvent::Message { text, data }
        }
        WebSocketEvent::Close {
            code,
            reason,
            clean,
        } => NativeWebSocketEvent::Close {
            code,
            reason,
            clean,
        },
        WebSocketEvent::Error(message) => {
            let error = SocketError(message);
            NativeWebSocketEvent::Error {
                code: Some(error.code().to_owned()),
                message: error.0,
            }
        }
    }
}

fn make_close_message(code: Option<u16>, reason: String) -> Message {
    if code.is_none() && reason.is_empty() {
        return Message::Close(None);
    }
    Message::Close(Some(CloseFrame {
        code: CloseCode::from(code.unwrap_or(1000)),
        reason: reason.into(),
    }))
}

pub(crate) fn validate_ws_url(url: &str) -> Result<Uri, String> {
    let uri = url
        .parse::<Uri>()
        .map_err(|_| "unsupported WebSocket URL".to_owned())?;
    let Some(scheme) = uri.scheme_str() else {
        return Err("unsupported WebSocket URL".to_owned());
    };
    if !scheme.eq_ignore_ascii_case("ws") && !scheme.eq_ignore_ascii_case("wss") {
        return Err("unsupported WebSocket URL".to_owned());
    }
    if uri.authority().is_none_or(|auth| auth.host().is_empty()) {
        return Err("unsupported WebSocket URL".to_owned());
    }
    Ok(uri)
}

pub(crate) fn validate_subprotocols(protocols: &[String]) -> Result<(), String> {
    for protocol in protocols {
        if protocol.is_empty()
            || protocol
                .bytes()
                .any(|b| b <= 0x20 || b >= 0x7f || b == b',' || b == b';')
        {
            return Err(format!("invalid WebSocket subprotocol: {protocol:?}"));
        }
    }
    Ok(())
}

pub(crate) fn validate_close_args(code: Option<u16>, reason: &str) -> Result<(), String> {
    if let Some(code) = code
        && code != 1000
        && !(3000..=4999).contains(&code)
    {
        return Err(format!("invalid WebSocket close code: {code}"));
    }
    if reason.len() > 123 {
        return Err("WebSocket close reason must be <= 123 UTF-8 bytes".to_owned());
    }
    Ok(())
}

fn build_ws_request(url: &str, protocols: &[String]) -> Result<http::Request<()>, String> {
    validate_ws_url(url)?;
    validate_subprotocols(protocols)?;
    let mut request = url
        .into_client_request()
        .map_err(|err| format!("invalid WebSocket request: {err}"))?;
    if !protocols.is_empty() {
        let value = http::HeaderValue::from_str(&protocols.join(", "))
            .map_err(|err| format!("invalid WebSocket subprotocols: {err}"))?;
        request
            .headers_mut()
            .insert("sec-websocket-protocol", value);
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn non_ws_schemes_and_invalid_close_codes_are_rejected() {
        assert!(validate_ws_url("http://example.test/ws").is_err());
        assert!(validate_ws_url("ftp://example.test/ws").is_err());
        assert!(validate_ws_url("ws://127.0.0.1:8080/chat").is_ok());
        assert!(validate_ws_url("wss://example.test/chat").is_ok());
        assert!(validate_close_args(Some(1000), "bye").is_ok());
        assert!(validate_close_args(Some(3001), "app").is_ok());
        assert!(validate_close_args(Some(1006), "").is_err());
        assert!(validate_close_args(None, &"x".repeat(124)).is_err());
    }

    #[test]
    fn socket_slot_tracker_enforces_max_sockets_and_releases_on_drop() {
        let tracker = SocketSlotTracker::default();
        let mut held = Vec::new();
        for _ in 0..MAX_SOCKETS {
            held.push(tracker.acquire().expect("slot available"));
        }
        assert_eq!(tracker.active_count(), MAX_SOCKETS);
        assert!(tracker.acquire().is_err());
        held.pop();
        assert_eq!(tracker.active_count(), MAX_SOCKETS - 1);
        assert!(tracker.acquire().is_ok());
    }

    #[test]
    fn native_async_websocket_echo_and_clean_close() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let handle = runtime.handle();
        let addr = handle.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind ws test server");
            let addr = listener.local_addr().expect("local addr");
            tokio::spawn(async move {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
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
            addr
        });

        let request = build_ws_request(&format!("ws://{addr}/socket"), &[]).expect("ws request");
        handle.block_on(async move {
            let (mut ws, _) = connect_async_with_config(request, None, false)
                .await
                .expect("connect");
            ws.send(Message::Text("hi".into()))
                .await
                .expect("send text");
            let reply = ws.next().await.expect("frame").expect("ok");
            assert_eq!(reply, Message::Text("echo:hi".into()));
            ws.send(make_close_message(Some(1000), "done".into()))
                .await
                .expect("close");
            let closed = ws.next().await.expect("close frame").expect("ok");
            assert!(matches!(closed, Message::Close(_)));
        });
    }
}
