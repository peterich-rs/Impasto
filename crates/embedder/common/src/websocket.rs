//! Native async WebSocket runner and platform transport contract.

use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::time::Duration;

use bytes::Bytes;
use engine::session::{CancelToken, PluginValue, StreamSendError, StreamSink};
use futures_util::future::{Either, select};
use futures_util::{SinkExt as _, StreamExt as _};
use http::Uri;
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, WebSocketConfig};

use crate::diagnostics::{DiagnosticsHub, NetEntry, NetKind, PREVIEW_CAP};

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

pub trait WebSocketTransport: Send + Sync {
    fn run(
        &self,
        url: &str,
        protocols: &[String],
        commands: std_mpsc::Receiver<WebSocketCommand>,
        active: &dyn Fn() -> bool,
        emit: &mut dyn FnMut(WebSocketEvent) -> bool,
    ) -> Result<(), String>;
}

pub(crate) struct SocketSlotTracker(Arc<AtomicUsize>);

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
    rx: mpsc::Receiver<QueuedCommand>,
    transport: Option<Arc<dyn WebSocketTransport>>,
    devtools: Option<Arc<DiagnosticsHub>>,
}

#[derive(Debug, Clone)]
struct CloseRequest {
    code: Option<u16>,
    reason: String,
}

type PendingClose = Arc<Mutex<Option<CloseRequest>>>;

pub(crate) struct SocketSession {
    pub(crate) id: u64,
    tx: mpsc::Sender<QueuedCommand>,
    buffered_bytes: Arc<AtomicUsize>,
    ready_state: Arc<AtomicU8>,
    protocol: Arc<Mutex<String>>,
    cancel: CancelToken,
    close_notify: Arc<Notify>,
    pending_close: PendingClose,
    pending_run: Mutex<Option<(SlotTicket, PendingSocketRun)>>,
}

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

impl SocketSession {
    pub(crate) fn open(
        url: String,
        protocols: Vec<String>,
        tracker: &SocketSlotTracker,
        transport: Option<Arc<dyn WebSocketTransport>>,
        devtools: Option<Arc<DiagnosticsHub>>,
    ) -> Result<Arc<Self>, String> {
        validate_ws_url(&url)?;
        validate_subprotocols(&protocols)?;
        let ticket = tracker.acquire()?;
        let (tx, rx) = mpsc::channel(CMD_CHANNEL_CAP);
        let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed) + 1;

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
                    rx,
                    transport,
                    devtools,
                },
            ))),
        }))
    }

    pub(crate) fn ready_state(&self) -> u8 {
        self.ready_state.load(Ordering::Acquire)
    }

    pub(crate) fn buffered_amount(&self) -> usize {
        self.buffered_bytes.load(Ordering::Acquire)
    }

    pub(crate) fn protocol(&self) -> String {
        self.protocol
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(crate) fn send_text(&self, text: String) -> Result<usize, String> {
        self.enqueue_data(true, text.into_bytes())
    }

    pub(crate) fn send_binary(&self, data: Vec<u8>) -> Result<usize, String> {
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

    pub(crate) fn close(&self, code: Option<u16>, reason: String) -> Result<(), String> {
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

    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
        self.close_notify.notify_waiters();
        let _ = self
            .pending_run
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
    }

    pub(crate) fn start_stream(&self, sink: StreamSink) -> Result<(), Box<(StreamSink, String)>> {
        let Some((ticket, run)) = self
            .pending_run
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        else {
            return Err(Box::new((
                sink,
                "WebSocket event stream was already started or closed".to_owned(),
            )));
        };
        let handle = match crate::rt::handle() {
            Ok(handle) => handle,
            Err(error) => return Err(Box::new((sink, error))),
        };
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
        drop(handle.spawn(run_socket(run, shared, sink, handle.clone())));
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

    fn record_event(&self, devtools: Option<&DiagnosticsHub>, url: &str, event: &WebSocketEvent) {
        let Some(hub) = devtools else {
            return;
        };
        let at = crate::timing::mono_ms();
        let (method, error, preview) = match event {
            WebSocketEvent::Open { .. } => ("WS OPEN", None, None),
            WebSocketEvent::Message { data, .. } => (
                "WS MSG",
                None,
                Some(data.iter().copied().take(PREVIEW_CAP).collect()),
            ),
            WebSocketEvent::Close { code, reason, .. } => (
                "WS CLOSE",
                (!reason.is_empty()).then(|| format!("{code} {reason}")),
                None,
            ),
            WebSocketEvent::Error(msg) => ("WS ERROR", Some(msg.clone()), None),
        };
        let response_bytes = match event {
            WebSocketEvent::Message { data, .. } => Some(data.len()),
            _ => None,
        };
        hub.record_net(NetEntry {
            seq: 0,
            at_ms: at,
            kind: NetKind::Socket,
            token: self.id,
            method: method.to_owned(),
            url: url.to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            status: None,
            response_headers: Vec::new(),
            response_bytes,
            response_preview: preview,
            error,
            done_ms: Some(at),
        });
    }
}

async fn run_socket(
    run: PendingSocketRun,
    shared: SocketShared,
    mut sink: StreamSink,
    handle: tokio::runtime::Handle,
) {
    let PendingSocketRun {
        url,
        protocols,
        rx,
        transport,
        devtools,
    } = run;
    if let Some(platform) = transport {
        run_blocking_transport(platform, url, protocols, rx, shared, sink, handle).await;
        return;
    }

    run_native_async(url, protocols, rx, &shared, &mut sink, devtools).await;
    shared.mark_closed();
    sink.finish();
}

async fn run_native_async(
    url: String,
    protocols: Vec<String>,
    mut rx: mpsc::Receiver<QueuedCommand>,
    shared: &SocketShared,
    sink: &mut StreamSink,
    devtools: Option<Arc<DiagnosticsHub>>,
) {
    let request = match build_ws_request(&url, &protocols) {
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

    loop {
        if shared.is_cancelled() {
            let _ = ws_tx.close().await;
            break;
        }

        enum Step {
            Cancel,
            Incoming(Option<Result<Message, tokio_tungstenite::tungstenite::Error>>),
            Command(Option<QueuedCommand>),
            CloseTimeout,
        }

        let step = {
            let cancelled = std::pin::pin!(shared.cancelled());
            let next_msg = std::pin::pin!(ws_rx.next());
            let next_cmd = std::pin::pin!(rx.recv());
            match close_deadline {
                Some(deadline) => {
                    let sleep = std::pin::pin!(tokio::time::sleep_until(deadline));
                    match select(select(cancelled, sleep), next_msg).await {
                        Either::Left((Either::Left(((), _)), _)) => Step::Cancel,
                        Either::Left((Either::Right(((), _)), _)) => Step::CloseTimeout,
                        Either::Right((msg, _)) => Step::Incoming(msg),
                    }
                }
                None => match select(cancelled, select(next_msg, next_cmd)).await {
                    Either::Left(((), _)) => Step::Cancel,
                    Either::Right((Either::Left((msg, _)), _)) => Step::Incoming(msg),
                    Either::Right((Either::Right((cmd, _)), _)) => Step::Command(cmd),
                },
            }
        };

        match step {
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
                            Ok(msg) => ws_tx.send(msg).await.map_err(|e| e.to_string()),
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
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            },
        }
    }
}

async fn run_blocking_transport(
    transport: Arc<dyn WebSocketTransport>,
    url: String,
    protocols: Vec<String>,
    mut rx: mpsc::Receiver<QueuedCommand>,
    shared: SocketShared,
    mut sink: StreamSink,
    handle: tokio::runtime::Handle,
) {
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
        let outcome = transport.run(
            &url,
            &protocols,
            std_rx,
            &|| !active_cancel.is_cancelled() && !active_sink_cancel.is_cancelled(),
            &mut |event| {
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
                let Some(item) = encode_event(event) else {
                    return false;
                };
                sink.send(item, BLOCKING_SEND_TIMEOUT).is_ok()
            },
        );
        shared.mark_closed();
        if let Err(message) = outcome
            && !emit_cancel.is_cancelled()
            && !emit_sink_cancel.is_cancelled()
            && let Some(item) = encode_event(WebSocketEvent::Error(message))
        {
            let _ = sink.send(item, BLOCKING_SEND_TIMEOUT);
        }
        sink.finish();
    });

    let _ = join.await;
    forward_task.abort();
}

async fn emit_error_and_close(
    sink: &mut StreamSink,
    shared: &SocketShared,
    devtools: Option<&DiagnosticsHub>,
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

async fn send_event_async(sink: &mut StreamSink, event: WebSocketEvent) -> bool {
    let Some(value) = encode_event(event) else {
        return false;
    };
    !matches!(sink.send_async(value).await, Err(StreamSendError::Closed))
}

fn encode_event(event: WebSocketEvent) -> Option<PluginValue> {
    Some(match event {
        WebSocketEvent::Open { protocol } => PluginValue::Map(vec![
            ("type".to_owned(), PluginValue::Str("open".to_owned())),
            ("protocol".to_owned(), PluginValue::Str(protocol)),
        ]),
        WebSocketEvent::Message { text, data } => {
            let payload = if text {
                match String::from_utf8(data) {
                    Ok(s) => PluginValue::Str(s),
                    Err(e) => PluginValue::Bytes(e.into_bytes()),
                }
            } else {
                PluginValue::Bytes(data)
            };
            PluginValue::Map(vec![
                ("type".to_owned(), PluginValue::Str("message".to_owned())),
                ("text".to_owned(), PluginValue::Bool(text)),
                ("data".to_owned(), payload),
            ])
        }
        WebSocketEvent::Close {
            code,
            reason,
            clean,
        } => PluginValue::Map(vec![
            ("type".to_owned(), PluginValue::Str("close".to_owned())),
            ("code".to_owned(), PluginValue::F64(f64::from(code))),
            ("reason".to_owned(), PluginValue::Str(reason)),
            ("clean".to_owned(), PluginValue::Bool(clean)),
        ]),
        WebSocketEvent::Error(message) => PluginValue::Map(vec![
            ("type".to_owned(), PluginValue::Str("error".to_owned())),
            ("message".to_owned(), PluginValue::Str(message)),
        ]),
    })
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
        let handle = crate::rt::handle().expect("runtime");
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
