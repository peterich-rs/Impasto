//! Foreground HTTP through the system URL loading stack.

#![allow(
    non_snake_case,
    reason = "delegate method names match the Objective-C protocol"
)]

use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

use crate::options::{CookieJar, Pins, ProxyOptions};
use crate::wire::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    RedirectMode,
};
use block2::DynBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSNumber,
    NSString, NSURL, NSURLAuthenticationChallenge, NSURLCredential, NSURLCredentialPersistence,
    NSURLErrorBadURL as BAD_URL, NSURLErrorCancelled as CANCELLED, NSURLErrorTimedOut as TIMED_OUT,
    NSURLErrorUnsupportedURL as UNSUPPORTED_URL, NSURLRequest, NSURLRequestCachePolicy,
    NSURLResponse, NSURLSession, NSURLSessionAuthChallengeDisposition as AuthDisposition,
    NSURLSessionConfiguration, NSURLSessionDataDelegate, NSURLSessionDataTask,
    NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate,
};

pub(super) struct Http {
    session: Session,
    proxy: Option<url::Url>,
}

impl Http {
    pub(super) fn new(proxy: Option<&ProxyOptions>) -> Result<Self, HttpError> {
        autoreleasepool(|_| {
            let configuration = NSURLSessionConfiguration::ephemeralSessionConfiguration();
            configuration
                .setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
            configuration.setHTTPShouldSetCookies(false);
            let proxy = proxy
                .map(|proxy| configure_proxy(&configuration, proxy))
                .transpose()?;
            let session = Session(NSURLSession::sessionWithConfiguration(&configuration));
            Ok(Self { session, proxy })
        })
    }
}

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
    Upload {
        loaded: u64,
        total: Option<u64>,
    },
    Completed(Result<(), HttpError>),
}

struct ReceiverState {
    send: SyncSender<Received>,
    redirects: AtomicUsize,
    redirect_mode: RedirectMode,
    max_bytes: usize,
    bytes: AtomicUsize,
    cookies: Option<Arc<CookieJar>>,
    pins: Option<Arc<Pins>>,
    proxy: Option<url::Url>,
    progress: bool,
    reported: AtomicU64,
    explicit_cookies: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ImpastoHttpPackageDataDelegate"]
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
            response: &NSHTTPURLResponse,
            request: &NSURLRequest,
            completion: &DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            let state = self.ivars();
            self.store_cookies(response);
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
            let Some(url) = request.URL() else {
                completion.call((ptr::null_mut(),));
                return;
            };
            if reject_plaintext(&url, state.pins.as_deref()) {
                let _ = state.send.send(Received::Completed(Err(pin_mismatch())));
                completion.call((ptr::null_mut(),));
                task.cancel();
                return;
            }
            // SAFETY: mutableCopy for a URL request returns an owned NSMutableURLRequest.
            let next: Retained<NSMutableURLRequest> = unsafe { msg_send![request, mutableCopy] };
            if origin(response.URL().as_deref()) != origin(Some(&url)) {
                for name in ["Authorization", "Cookie", "Proxy-Authorization"] {
                    next.setValue_forHTTPHeaderField(None, &NSString::from_str(name));
                }
            }
            if state.cookies.is_some() && !state.explicit_cookies {
                next.setValue_forHTTPHeaderField(None, &NSString::from_str("Cookie"));
            }
            attach_cookies(&next, &url, state.cookies.as_deref());
            completion.call((ptr::from_ref(&**next).cast_mut(),));
        }

        #[unsafe(method(URLSession:task:didReceiveChallenge:completionHandler:))]
        unsafe fn URLSession_task_didReceiveChallenge_completionHandler(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionTask,
            challenge: &NSURLAuthenticationChallenge,
            completion: &Challenge,
        ) {
            self.challenge(challenge, completion);
        }

        #[unsafe(method(URLSession:task:didSendBodyData:totalBytesSent:totalBytesExpectedToSend:))]
        fn URLSession_task_didSendBodyData_totalBytesSent_totalBytesExpectedToSend(
            &self,
            _: &NSURLSession,
            _: &NSURLSessionTask,
            _: i64,
            loaded: i64,
            total: i64,
        ) {
            let state = self.ivars();
            if !state.progress {
                return;
            }
            let Ok(loaded) = u64::try_from(loaded) else {
                return;
            };
            let total = u64::try_from(total).ok();
            let previous = state.reported.load(Ordering::Relaxed);
            if Some(loaded) == total || loaded.saturating_sub(previous) >= 64 * 1024 {
                state.reported.store(loaded, Ordering::Relaxed);
                let _ = state.send.send(Received::Upload { loaded, total });
            }
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
                    .and_then(|response| {
                        self.store_cookies(response);
                        response_head(response)
                    });
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
        request: &HttpRequest,
        proxy: Option<url::Url>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ReceiverState {
            send,
            redirects: AtomicUsize::new(0),
            redirect_mode: request.redirect,
            max_bytes: request.max_body_bytes.unwrap_or(HTTP_BODY_LIMIT),
            bytes: AtomicUsize::new(0),
            cookies: request.extra.cookies.clone(),
            pins: request.extra.pins.clone(),
            proxy,
            progress: request.extra.progress,
            reported: AtomicU64::new(0),
            explicit_cookies: request
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("cookie")),
        });
        // SAFETY: this is an allocated NSObject subclass initialized once.
        unsafe { msg_send![super(this), init] }
    }
}

type Challenge = DynBlock<dyn Fn(AuthDisposition, *mut NSURLCredential)>;

impl Receiver {
    fn store_cookies(&self, response: &NSHTTPURLResponse) {
        if let Some(jar) = &self.ivars().cookies
            && let Some(Received::Head {
                url: Some(url),
                headers,
                ..
            }) = response_head(response)
        {
            jar.update(&url, &headers);
        }
    }

    fn challenge(&self, challenge: &NSURLAuthenticationChallenge, completion: &Challenge) {
        let space = challenge.protectionSpace();
        let host = space.host().to_string();
        if space.isProxy()
            && let Some(proxy) = &self.ivars().proxy
            && proxy.host_str() == Some(host.as_str())
            && !proxy.username().is_empty()
            && challenge.previousFailureCount() == 0
        {
            let credential = NSURLCredential::credentialWithUser_password_persistence(
                &NSString::from_str(proxy.username()),
                &NSString::from_str(proxy.password().unwrap_or_default()),
                NSURLCredentialPersistence::None,
            );
            completion.call((
                AuthDisposition::UseCredential,
                Retained::as_ptr(&credential).cast_mut(),
            ));
            return;
        }
        let pins = self
            .ivars()
            .pins
            .as_deref()
            .filter(|pins| pins.required(&host));
        if space.authenticationMethod().to_string() != "NSURLAuthenticationMethodServerTrust"
            || pins.is_none()
        {
            completion.call((AuthDisposition::PerformDefaultHandling, ptr::null_mut()));
            return;
        }
        if let Some(pins) = pins
            && let Some(credential) = trusted_credential(&space, &host, pins)
        {
            completion.call((
                AuthDisposition::UseCredential,
                Retained::as_ptr(&credential).cast_mut(),
            ));
        } else {
            let _ = self
                .ivars()
                .send
                .send(Received::Completed(Err(pin_mismatch())));
            completion.call((
                AuthDisposition::CancelAuthenticationChallenge,
                ptr::null_mut(),
            ));
        }
    }
}

#[repr(C)]
struct NativeSecTrust {
    opaque: [u8; 0],
}

// SAFETY: Security.h declares SecTrustRef as a pointer to the opaque __SecTrust structure.
unsafe impl objc2::encode::RefEncode for NativeSecTrust {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("__SecTrust", &[]));
}

fn trusted_credential(
    space: &objc2_foundation::NSURLProtectionSpace,
    host: &str,
    pins: &Pins,
) -> Option<Retained<NSURLCredential>> {
    use core_foundation::base::TCFType as _;
    use security_framework::trust::SecTrust;
    // SAFETY: serverTrust is the Security-framework SecTrustRef owned by the protection space.
    let raw: *mut NativeSecTrust = unsafe { msg_send![space, serverTrust] };
    if raw.is_null() {
        return None;
    }
    // SAFETY: the non-null borrowed SecTrustRef is retained for the lifetime of the wrapper.
    let trust = unsafe { SecTrust::wrap_under_get_rule(raw.cast()) };
    trust.evaluate_with_error().ok()?;
    let chain = trust
        .chain()
        .iter()
        .map(|certificate| certificate.to_der())
        .collect::<Vec<_>>();
    if !pins.verify(host, chain.iter().map(Vec::as_slice)) {
        return None;
    }
    // SAFETY: credentialForTrust: accepts the live evaluated trust and returns an autoreleased credential.
    unsafe { msg_send![objc2::class!(NSURLCredential), credentialForTrust: raw] }
}

fn origin(url: Option<&NSURL>) -> Option<url::Origin> {
    let url = url?.absoluteString()?.to_string();
    url::Url::parse(&url).ok().map(|url| url.origin())
}

fn attach_cookies(request: &NSMutableURLRequest, url: &NSURL, jar: Option<&CookieJar>) {
    let field = NSString::from_str("Cookie");
    if request.valueForHTTPHeaderField(&field).is_none()
        && let Some(jar) = jar
        && let Some(url) = url.absoluteString()
        && let Some(value) = jar.value(&url.to_string())
    {
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str(&value)), &field);
    }
}

fn reject_plaintext(url: &NSURL, pins: Option<&Pins>) -> bool {
    pins.is_some_and(|pins| {
        url.host()
            .is_some_and(|host| pins.required(&host.to_string()))
    }) && url
        .scheme()
        .is_some_and(|scheme| scheme.to_string() != "https")
}

fn pin_mismatch() -> HttpError {
    HttpError::new(HttpFailure::TlsPinMismatch, "HTTP certificate pin mismatch")
}

fn configure_proxy(
    configuration: &NSURLSessionConfiguration,
    options: &ProxyOptions,
) -> Result<url::Url, HttpError> {
    let parsed = url::Url::parse(&options.url)
        .ok()
        .filter(|url| url.scheme() == "http" && url.host().is_some())
        .ok_or_else(|| HttpError::new(HttpFailure::BadUrl, "Unsupported HTTP proxy URL"))?;
    let host: Retained<AnyObject> = NSString::from_str(parsed.host_str().unwrap_or_default())
        .into_super()
        .into_super();
    let port: Retained<AnyObject> = NSNumber::numberWithUnsignedShort(parsed.port().unwrap_or(80))
        .into_super()
        .into_super()
        .into_super();
    let enabled: Retained<AnyObject> = NSNumber::numberWithBool(true)
        .into_super()
        .into_super()
        .into_super();
    let bypass = options
        .bypass
        .iter()
        .flatten()
        .map(|rule| NSString::from_str(rule))
        .collect::<Vec<_>>();
    let exclusions: Retained<AnyObject> = NSArray::from_retained_slice(&bypass)
        .into_super()
        .into_super();
    let keys = [
        "HTTPEnable",
        "HTTPProxy",
        "HTTPPort",
        "HTTPSEnable",
        "HTTPSProxy",
        "HTTPSPort",
        "ExceptionsList",
    ]
    .map(NSString::from_str);
    let values = [
        enabled.clone(),
        host.clone(),
        port.clone(),
        enabled,
        host,
        port,
        exclusions,
    ];
    let dict = NSDictionary::from_retained_objects(&keys.each_ref().map(|key| &**key), &values);
    // SAFETY: Objective-C lightweight generics are erased; all keys and values are objects.
    let dict = unsafe {
        Retained::<NSDictionary<NSString, AnyObject>>::cast_unchecked::<NSDictionary>(dict)
    };
    // SAFETY: CFNetwork accepts NSString keys and NSNumber, NSString, or NSArray values.
    unsafe { configuration.setConnectionProxyDictionary(Some(&dict)) };
    Ok(parsed)
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

struct RunningTask(Retained<NSURLSessionTask>);

impl Drop for RunningTask {
    fn drop(&mut self) {
        self.0.cancel();
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
                HttpPiece::Upload { .. } => {}
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
            let native = native_request(request, timeout)?;
            let (send, receive) = mpsc::sync_channel(2);
            let delegate = Receiver::new(send, request, self.proxy.clone());
            let task: Retained<NSURLSessionTask> = if let Some(path) = &request.extra.upload {
                let path = path
                    .to_str()
                    .ok_or_else(|| HttpError::new(HttpFailure::Io, "Upload path is not UTF-8"))?;
                let file = NSURL::fileURLWithPath(&NSString::from_str(path));
                self.session
                    .0
                    .uploadTaskWithRequest_fromFile(&native, &file)
                    .into_super()
                    .into_super()
            } else {
                self.session.0.dataTaskWithRequest(&native).into_super()
            };
            task.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            let task = RunningTask(task);
            task.0.resume();
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
                    Received::Upload { loaded, total } => {
                        if !sink(HttpPiece::Upload { loaded, total }) {
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
    if reject_plaintext(&url, request.extra.pins.as_deref()) {
        return Err(pin_mismatch());
    }
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
    attach_cookies(&native, &url, request.extra.cookies.as_deref());
    if request.extra.upload.is_none()
        && (!request.body.is_empty() || !matches!(request.method.as_str(), "GET" | "HEAD"))
    {
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

    use impasto::CancelToken;

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
            extra: Default::default(),
            max_body_bytes: None,
        };
        let mut status = 0;
        let mut body = Vec::new();
        let mut done = false;
        let client = Http::new(None).map_err(|error| error.message)?;
        let transfer = client.stream(&request, &mut |piece| {
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
