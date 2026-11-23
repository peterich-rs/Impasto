//! Foreground HTTP through the system URL loading stack.

#![allow(
    non_snake_case,
    reason = "delegate method names match the Objective-C protocol"
)]

use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use block2::DynBlock;
use embedder::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    RedirectMode,
};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL,
    NSURLErrorBadURL as BAD_URL, NSURLErrorCancelled as CANCELLED, NSURLErrorTimedOut as TIMED_OUT,
    NSURLErrorUnsupportedURL as UNSUPPORTED_URL, NSURLRequest, NSURLRequestCachePolicy,
    NSURLResponse, NSURLSession, NSURLSessionConfiguration, NSURLSessionDataDelegate,
    NSURLSessionDataTask, NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate,
};

pub(super) struct Http;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
const CANCEL_CHECK: Duration = Duration::from_millis(20);
const REDIRECT_LIMIT: usize = 5;

enum Received {
    Head {
        status: u16,
        url: Option<String>,
        headers: Vec<(String, String)>,
    },
    Body(Vec<u8>),
    Completed(Result<(), HttpError>),
}

struct ReceiverState {
    send: SyncSender<Received>,
    redirects: AtomicUsize,
    redirect_mode: RedirectMode,
    max_bytes: usize,
    bytes: AtomicUsize,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ImpastoURLSessionDataDelegate"]
    #[ivars = ReceiverState]
    struct Receiver;

    // SAFETY: Receiver is an NSObject subclass with thread-safe ivars.
    unsafe impl NSObjectProtocol for Receiver {}
    // SAFETY: All delegate state can be read on the session delegate queue.
    unsafe impl NSURLSessionDelegate for Receiver {}

    // SAFETY: The selectors and Objective-C encodings are those of NSURLSessionTaskDelegate.
    unsafe impl NSURLSessionTaskDelegate for Receiver {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        unsafe fn URLSession_task_willPerformHTTPRedirection_newRequest_completionHandler(
            &self,
            _: &NSURLSession,
            task: &NSURLSessionTask,
            _: &NSHTTPURLResponse,
            request: &NSURLRequest,
            completion: &DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            let state = self.ivars();
            match state.redirect_mode {
                RedirectMode::Manual => {
                    completion.call((ptr::null_mut(),));
                    return;
                }
                RedirectMode::Error => {
                    let _ = state.send.send(Received::Completed(Err(HttpError::new(
                        HttpFailure::Network,
                        "redirect disallowed by redirect mode",
                    ))));
                    completion.call((ptr::null_mut(),));
                    task.cancel();
                    return;
                }
                RedirectMode::Follow => {}
            }
            let count = state.redirects.fetch_add(1, Ordering::Relaxed);
            if count >= REDIRECT_LIMIT || request.URL().is_none_or(|url| !is_http_url(&url)) {
                let message = if count >= REDIRECT_LIMIT {
                    "too many HTTP redirects"
                } else {
                    "unsupported redirect URL"
                };
                let _ = state.send.send(Received::Completed(Err(HttpError::new(
                    HttpFailure::Network,
                    message,
                ))));
                completion.call((ptr::null_mut(),));
                task.cancel();
                return;
            }
            completion.call((ptr::from_ref(request).cast_mut(),));
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn URLSession_task_didCompleteWithError(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            let result = error.map_or(Ok(()), |error| Err(http_error(error)));
            let _ = self.ivars().send.send(Received::Completed(result));
        }
    }

    // SAFETY: The selectors and Objective-C encodings are those of NSURLSessionDataDelegate.
    unsafe impl NSURLSessionDataDelegate for Receiver {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        unsafe fn URLSession_dataTask_didReceiveResponse_completionHandler(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionDataTask,
            response: &NSURLResponse,
            completion: &DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            autoreleasepool(|_| {
                let response: &AnyObject = response;
                let result = response
                    .downcast_ref::<NSHTTPURLResponse>()
                    .and_then(response_head);
                let disposition = match result {
                    Some(head) => {
                        if self.ivars().send.send(head).is_ok() {
                            NSURLSessionResponseDisposition::Allow
                        } else {
                            NSURLSessionResponseDisposition::Cancel
                        }
                    }
                    None => {
                        let _ = self
                            .ivars()
                            .send
                            .send(Received::Completed(Err(HttpError::new(
                                HttpFailure::Network,
                                "invalid HTTP response",
                            ))));
                        NSURLSessionResponseDisposition::Cancel
                    }
                };
                completion.call((disposition,));
            });
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn URLSession_dataTask_didReceiveData(
            &self,
            _: &NSURLSession,
            task: &NSURLSessionDataTask,
            data: &NSData,
        ) {
            let state = self.ivars();
            let limit = state
                .bytes
                .fetch_add(data.len(), Ordering::Relaxed)
                .saturating_add(data.len());
            let exceeded = limit > state.max_bytes;
            let received = if exceeded {
                Received::Completed(Err(HttpError::new(
                    HttpFailure::TooLarge,
                    "response body too large",
                )))
            } else {
                Received::Body(data.to_vec())
            };
            if state.send.send(received).is_err() || exceeded {
                task.cancel();
            }
        }
    }
);

impl Receiver {
    fn new(
        send: SyncSender<Received>,
        redirect_mode: RedirectMode,
        max_bytes: usize,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ReceiverState {
            send,
            redirects: AtomicUsize::new(0),
            redirect_mode,
            max_bytes,
            bytes: AtomicUsize::new(0),
        });
        // SAFETY: this is an allocated NSObject subclass initialized once.
        unsafe { msg_send![super(this), init] }
    }
}

fn response_head(response: &NSHTTPURLResponse) -> Option<Received> {
    let status = u16::try_from(response.statusCode()).ok()?;
    let url = response
        .URL()
        .and_then(|url| url.absoluteString())
        .map(|s| s.to_string());
    let fields = response.allHeaderFields();
    let headers = fields
        .keys()
        .filter_map(|key| {
            let name = key
                .downcast_ref::<NSString>()?
                .to_string()
                .to_ascii_lowercase();
            let value = fields.objectForKey(&key)?;
            let value = value.downcast_ref::<NSString>()?.to_string();
            Some((name, value))
        })
        .collect();
    Some(Received::Head {
        status,
        url,
        headers,
    })
}

fn http_error(error: &NSError) -> HttpError {
    let kind = match error.code() {
        CANCELLED => HttpFailure::Canceled,
        TIMED_OUT => HttpFailure::Timeout,
        BAD_URL | UNSUPPORTED_URL => HttpFailure::BadUrl,
        _ => HttpFailure::Network,
    };
    HttpError::new(kind, error.localizedDescription().to_string())
}

struct Session(Retained<NSURLSession>);

impl Drop for Session {
    fn drop(&mut self) {
        self.0.invalidateAndCancel();
    }
}

impl HttpClient for Http {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut head = None;
        let mut body = Vec::new();
        self.stream(request, &mut |piece| {
            match piece {
                HttpPiece::Head {
                    status,
                    url,
                    redirected,
                    headers,
                } => head = Some((status, url.to_owned(), redirected, headers.to_vec())),
                HttpPiece::Chunk(bytes, _) => body.extend_from_slice(bytes),
            }
            true
        })?;
        let (status, url, redirected, headers) =
            head.ok_or_else(|| HttpError::new(HttpFailure::Network, "no HTTP response"))?;
        Ok(HttpResponse {
            status,
            url,
            redirected,
            headers,
            body,
        })
    }

    fn stream(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        autoreleasepool(|_| {
            let timeout = request
                .timeout_ms
                .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
            if request.cancel.is_cancelled() {
                return Err(aborted());
            }
            let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
            configuration.setTimeoutIntervalForRequest(timeout.as_secs_f64());
            configuration.setTimeoutIntervalForResource(timeout.as_secs_f64());
            configuration
                .setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
            configuration.setHTTPShouldSetCookies(false);
            let native = native_request(request, timeout)?;
            let (send, receive) = mpsc::sync_channel(2);
            let delegate = Receiver::new(
                send,
                request.redirect,
                request.max_body_bytes.unwrap_or(HTTP_BODY_LIMIT),
            );
            // SAFETY: NSURLSession creates its own serial delegate queue; the
            // receiver owns all state by value, and its ivars are Send + Sync.
            let session = Session(unsafe {
                NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                    &configuration,
                    Some(ProtocolObject::from_ref(&*delegate)),
                    None,
                )
            });
            let task = session.0.dataTaskWithRequest(&native);
            task.resume();
            let start = Instant::now();
            loop {
                if request.cancel.is_cancelled() {
                    return Err(aborted());
                }
                let left = timeout.saturating_sub(start.elapsed());
                if left.is_zero() {
                    return Err(HttpError::new(
                        HttpFailure::Timeout,
                        "iOS HTTP deadline elapsed",
                    ));
                }
                let received = match receive.recv_timeout(CANCEL_CHECK.min(left)) {
                    Ok(received) => received,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(HttpError::new(
                            HttpFailure::Network,
                            "iOS HTTP response ended early",
                        ));
                    }
                };
                if request.cancel.is_cancelled() {
                    return Err(aborted());
                }
                match received {
                    Received::Head {
                        status,
                        url,
                        headers,
                    } => {
                        let url = url.as_deref().unwrap_or(&request.url);
                        let redirected = delegate.ivars().redirects.load(Ordering::Relaxed) > 0;
                        if !sink(HttpPiece::Head {
                            status,
                            url,
                            redirected,
                            headers: &headers,
                        }) {
                            return Ok(());
                        }
                    }
                    Received::Body(bytes) => {
                        if !sink(HttpPiece::Chunk(&bytes, false)) {
                            return Ok(());
                        }
                    }
                    Received::Completed(result) => {
                        result?;
                        sink(HttpPiece::Chunk(&[], true));
                        return Ok(());
                    }
                }
            }
        })
    }
}

fn native_request(
    request: &HttpRequest,
    timeout: Duration,
) -> Result<Retained<NSMutableURLRequest>, HttpError> {
    let url = NSURL::URLWithString(&NSString::from_str(&request.url))
        .filter(|url| is_http_url(url))
        .ok_or_else(|| HttpError::new(HttpFailure::BadUrl, "unsupported HTTP URL"))?;
    if !is_token(&request.method) {
        return Err(HttpError::new(
            HttpFailure::BadUrl,
            "unsupported HTTP method",
        ));
    }
    let native = NSMutableURLRequest::requestWithURL_cachePolicy_timeoutInterval(
        &url,
        NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData,
        timeout.as_secs_f64(),
    );
    native.setHTTPMethod(&NSString::from_str(&request.method));
    for (name, value) in &request.headers {
        if !is_token(name)
            || value
                .bytes()
                .any(|byte| byte == 127 || byte < 32 && byte != b'\t')
        {
            return Err(HttpError::new(HttpFailure::BadUrl, "invalid HTTP header"));
        }
        if hop_by_hop(name) {
            continue;
        }
        native.addValue_forHTTPHeaderField(&NSString::from_str(value), &NSString::from_str(name));
    }
    if !request.body.is_empty() || !matches!(request.method.as_str(), "GET" | "HEAD") {
        native.setHTTPBody(Some(&NSData::with_bytes(&request.body)));
    }
    Ok(native)
}

fn is_http_url(url: &NSURL) -> bool {
    matches!(
        url.scheme().as_deref().map(ToString::to_string).as_deref(),
        Some("http" | "https")
    ) && url.host().is_some()
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^`_|~".contains(&byte))
}

fn hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

fn aborted() -> HttpError {
    HttpError::new(HttpFailure::Canceled, "AbortError")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use engine::session::CancelToken;

    use super::*;

    #[test]
    fn native_http_stream_delivers_body_before_the_transfer_completes() -> Result<(), String> {
        const FIRST_BODY: usize = 128 * 1024;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let (consumed, observed) = mpsc::sync_channel::<()>(1);
        let server = thread::spawn(move || -> Result<(), String> {
            let started = Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(3) =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .map_err(|error| error.to_string())?;
            let mut input = [0; 1024];
            let _ = socket.read(&mut input).map_err(|error| error.to_string())?;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", FIRST_BODY + 3)
                .map_err(|error| error.to_string())?;
            socket
                .write_all(&vec![b'x'; FIRST_BODY])
                .map_err(|error| error.to_string())?;
            observed
                .recv_timeout(Duration::from_secs(3))
                .map_err(|error| error.to_string())?;
            socket
                .write_all(b"def")
                .map_err(|error| error.to_string())?;
            Ok(())
        });
        let request = HttpRequest {
            method: "GET".into(),
            url: format!("http://{address}"),
            headers: Vec::new(),
            body: Vec::new(),
            cancel: CancelToken::new(),
            timeout_ms: Some(4_000),
            redirect: RedirectMode::Follow,
            max_body_bytes: None,
        };
        let mut status = 0;
        let mut body = Vec::new();
        let mut done = false;
        let transfer = Http.stream(&request, &mut |piece| {
            match piece {
                HttpPiece::Head { status: code, .. } => status = code,
                HttpPiece::Chunk(bytes, last) => {
                    body.extend_from_slice(bytes);
                    if body.len() == FIRST_BODY && !last {
                        let _ = consumed.try_send(());
                    }
                    done |= last;
                }
            }
            true
        });
        let sent = server
            .join()
            .map_err(|_| "HTTP fixture panicked".to_owned())?;
        transfer.map_err(|error| error.message)?;
        sent?;
        assert_eq!(status, 200);
        assert_eq!(body.len(), FIRST_BODY + 3);
        assert!(body[..FIRST_BODY].iter().all(|byte| *byte == b'x'));
        assert_eq!(&body[FIRST_BODY..], b"def");
        assert!(done);
        Ok(())
    }
}
