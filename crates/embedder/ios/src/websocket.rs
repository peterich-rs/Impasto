//! Foreground WebSockets through NSURLSession with bounded driver queues.

#![allow(
    non_snake_case,
    reason = "delegate method names match the Objective-C protocol"
)]

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use block2::RcBlock;
use embedder::{WebSocketCommand, WebSocketEvent, WebSocketTransport};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSData, NSError, NSString, NSURL, NSURLSession, NSURLSessionConfiguration,
    NSURLSessionDelegate, NSURLSessionTask, NSURLSessionTaskDelegate,
    NSURLSessionWebSocketCloseCode, NSURLSessionWebSocketDelegate, NSURLSessionWebSocketMessage,
    NSURLSessionWebSocketTask,
};
use session::Credit;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_MESSAGE_BYTES: isize = 256 << 10;

pub(super) struct WebSocket;

enum NativeEvent {
    Open(String),
    Data(bool, Vec<u8>),
    Sent(Result<(), String>),
    Terminal(Result<(), String>),
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ImpastoURLSessionWebSocketDelegate"]
    #[ivars = Sender<NativeEvent>]
    struct Delegate;

    // SAFETY: Delegate is an NSObject subclass with a thread-safe sender.
    unsafe impl NSObjectProtocol for Delegate {}
    // SAFETY: Callbacks send owned events from the Foundation delegate queue.
    unsafe impl NSURLSessionDelegate for Delegate {}

    // SAFETY: The selectors and encodings are those of NSURLSessionTaskDelegate.
    unsafe impl NSURLSessionTaskDelegate for Delegate {
        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn URLSession_task_didCompleteWithError(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            let result = error.map_or(Ok(()), |error| Err(error.localizedDescription().to_string()));
            let _ = self.ivars().send(NativeEvent::Terminal(result));
        }
    }

    // SAFETY: The selectors and encodings are those of NSURLSessionWebSocketDelegate.
    unsafe impl NSURLSessionWebSocketDelegate for Delegate {
        #[unsafe(method(URLSession:webSocketTask:didOpenWithProtocol:))]
        fn URLSession_webSocketTask_didOpenWithProtocol(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionWebSocketTask,
            protocol: Option<&NSString>,
        ) {
            let protocol = protocol.map_or_else(String::new, ToString::to_string);
            let _ = self.ivars().send(NativeEvent::Open(protocol));
        }

        #[unsafe(method(URLSession:webSocketTask:didCloseWithCode:reason:))]
        fn URLSession_webSocketTask_didCloseWithCode_reason(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionWebSocketTask,
            _: NSURLSessionWebSocketCloseCode,
            _: Option<&NSData>,
        ) {
            let _ = self.ivars().send(NativeEvent::Terminal(Ok(())));
        }
    }
);

impl Delegate {
    fn new(send: Sender<NativeEvent>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(send);
        // SAFETY: this is an allocated NSObject subclass initialized once.
        unsafe { msg_send![super(this), init] }
    }
}

struct Connection {
    session: Retained<NSURLSession>,
    task: Retained<NSURLSessionWebSocketTask>,
}

impl Connection {
    fn new(url: &str, protocols: &[String], events: Sender<NativeEvent>) -> Result<Self, String> {
        let url = NSURL::URLWithString(&NSString::from_str(url))
            .filter(|url| {
                matches!(
                    url.scheme().as_deref().map(ToString::to_string).as_deref(),
                    Some("ws" | "wss")
                ) && url.host().is_some()
            })
            .ok_or_else(|| "invalid WebSocket URL".to_owned())?;
        let protocols: Vec<_> = protocols
            .iter()
            .map(|name| NSString::from_str(name))
            .collect();
        let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
        configuration.setTimeoutIntervalForRequest(CONNECT_TIMEOUT.as_secs_f64());
        configuration.setHTTPShouldSetCookies(false);
        let delegate = Delegate::new(events);
        // SAFETY: NSURLSession creates a serial delegate queue. Delegate owns a Send+Sync sender.
        let session = unsafe {
            NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                &configuration,
                Some(ProtocolObject::from_ref(&*delegate)),
                None,
            )
        };
        let task =
            session.webSocketTaskWithURL_protocols(&url, &NSArray::from_retained_slice(&protocols));
        task.setMaximumMessageSize(MAX_MESSAGE_BYTES);
        task.resume();
        Ok(Self { session, task })
    }

    fn receive(&self, sender: &Sender<NativeEvent>) {
        let sender = sender.clone();
        let handler = RcBlock::new(
            move |message: *mut NSURLSessionWebSocketMessage, error: *mut NSError| {
                // SAFETY: Foundation lends the message and error only for this callback.
                let (message, error) = unsafe { (message.as_ref(), error.as_ref()) };
                let event = if let Some(message) = message {
                    if let Some(text) = message.string() {
                        NativeEvent::Data(true, text.to_string().into_bytes())
                    } else if let Some(bytes) = message.data() {
                        NativeEvent::Data(false, bytes.to_vec())
                    } else {
                        NativeEvent::Terminal(Err("unknown iOS WebSocket message".into()))
                    }
                } else if let Some(error) = error {
                    NativeEvent::Terminal(Err(error.localizedDescription().to_string()))
                } else {
                    NativeEvent::Terminal(Ok(()))
                };
                let _ = sender.send(event);
            },
        );
        // SAFETY: the block only captures a Send+Sync sender and Foundation copies it.
        unsafe { self.task.receiveMessageWithCompletionHandler(&handler) }
    }

    fn send(&self, text: bool, data: Vec<u8>, sender: &Sender<NativeEvent>) -> Result<(), String> {
        let message = if text {
            let text = String::from_utf8(data).map_err(|error| error.to_string())?;
            NSURLSessionWebSocketMessage::initWithString(
                NSURLSessionWebSocketMessage::alloc(),
                &NSString::from_str(&text),
            )
        } else {
            NSURLSessionWebSocketMessage::initWithData(
                NSURLSessionWebSocketMessage::alloc(),
                &NSData::with_bytes(&data),
            )
        };
        let sender = sender.clone();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: Foundation lends the error for this callback.
            let error = unsafe { error.as_ref() };
            let _ = sender.send(NativeEvent::Sent(error.map_or(Ok(()), |error| {
                Err(error.localizedDescription().to_string())
            })));
        });
        // SAFETY: the block only captures a Send+Sync sender and Foundation copies it.
        unsafe { self.task.sendMessage_completionHandler(&message, &handler) }
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if self.task.closeCode() == NSURLSessionWebSocketCloseCode::Invalid {
            self.task
                .cancelWithCloseCode_reason(NSURLSessionWebSocketCloseCode::NormalClosure, None);
        }
        self.session.finishTasksAndInvalidate();
    }
}

impl WebSocketTransport for WebSocket {
    fn run(
        &self,
        url: &str,
        protocols: &[String],
        commands: Receiver<WebSocketCommand>,
        active: &dyn Fn() -> bool,
        emit: &dyn Fn(WebSocketEvent<'_>) -> bool,
    ) -> Result<(), String> {
        let (sender, receive) = mpsc::channel();
        let connection = autoreleasepool(|_| Connection::new(url, protocols, sender.clone()))?;
        let started = Instant::now();
        let mut opened = false;
        // The worker starts another send/receive only after its previous callback.
        // At most those two callbacks and the native lifecycle events can be queued.
        let mut sending: Option<(Instant, Option<Credit>)> = None;
        while active() {
            if !opened && started.elapsed() >= CONNECT_TIMEOUT {
                return Err("iOS WebSocket handshake timed out".into());
            }
            if sending
                .as_ref()
                .is_some_and(|(since, _)| since.elapsed() >= SEND_TIMEOUT)
            {
                return Err("iOS WebSocket send timed out".into());
            }
            if sending.is_none() {
                match commands.try_recv() {
                    Ok(WebSocketCommand::Send { text, data, credit }) => {
                        autoreleasepool(|_| connection.send(text, data, &sender))?;
                        sending = Some((Instant::now(), credit));
                    }
                    Ok(WebSocketCommand::Close) | Err(TryRecvError::Disconnected) => return Ok(()),
                    Err(TryRecvError::Empty) => {}
                }
            }
            match receive.recv_timeout(POLL_INTERVAL) {
                Ok(NativeEvent::Open(protocol)) => {
                    opened = true;
                    if !emit(WebSocketEvent::Opened(&protocol)) {
                        return Err("socket event queue is full".into());
                    }
                    autoreleasepool(|_| connection.receive(&sender));
                }
                Ok(NativeEvent::Data(text, data)) => {
                    if !emit(WebSocketEvent::Message { text, data: &data }) {
                        return Err("socket event queue is full".into());
                    }
                    autoreleasepool(|_| connection.receive(&sender));
                }
                Ok(NativeEvent::Sent(result)) => {
                    sending = None;
                    result?;
                }
                Ok(NativeEvent::Terminal(result)) => {
                    // A native receive may race with the delegate's normal peer-close notice.
                    if connection.task.closeCode() != NSURLSessionWebSocketCloseCode::Invalid {
                        return Ok(());
                    }
                    return result;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn native_websocket_sends_and_receives_owned_messages() -> Result<(), String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let server = thread::spawn(move || -> Result<(), String> {
            let started = Instant::now();
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(3) =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            stream
                .set_nonblocking(false)
                .map_err(|error| error.to_string())?;
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .map_err(|error| error.to_string())?;
            let mut socket = tungstenite::accept(stream).map_err(|error| error.to_string())?;
            let received = socket.read().map_err(|error| error.to_string())?;
            assert_eq!(
                received.to_text().map_err(|error| error.to_string())?,
                "hello"
            );
            socket
                .send(tungstenite::Message::Binary(vec![1, 2, 3].into()))
                .map_err(|error| error.to_string())?;
            let received = socket.read().map_err(|error| error.to_string())?;
            assert!(received.is_close());
            Ok(())
        });
        let (send, commands) = mpsc::channel();
        let messages = RefCell::new(Vec::new());
        let started = Instant::now();
        let result = WebSocket.run(
            &format!("ws://{address}"),
            &[],
            commands,
            &|| started.elapsed() < Duration::from_secs(4),
            &|event| {
                let command = match event {
                    WebSocketEvent::Opened(protocol) => {
                        assert!(protocol.is_empty());
                        WebSocketCommand::Send {
                            text: true,
                            data: b"hello".to_vec(),
                            credit: None,
                        }
                    }
                    WebSocketEvent::Message { text, data } => {
                        messages.borrow_mut().push((text, data.to_vec()));
                        WebSocketCommand::Close
                    }
                };
                send.send(command).is_ok()
            },
        );
        let server = server
            .join()
            .map_err(|_| "WebSocket fixture panicked".to_owned())?;
        result?;
        server?;
        assert_eq!(messages.into_inner(), [(false, vec![1, 2, 3])]);
        Ok(())
    }
}
