//! OpenWire transport, redirects and HTTP framing.

use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::future::{Either, select};
use http::{HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri};
use http_body_util::BodyExt as _;
use openwire::{Client, RequestBody, ResponseBody, WireError, WireErrorKind};

use impasto::CancelToken;

use super::{
    HTTP_BODY_LIMIT, HttpClient, HttpError, HttpFailure, HttpPiece, HttpRequest, HttpResponse,
    PublicError, RedirectMode, ResponseSink,
};

const MAX_REDIRECTS: usize = 20;

pub(crate) struct OpenWireClient {
    handle: tokio::runtime::Handle,
    client: Client,
}

impl OpenWireClient {
    pub fn new() -> Result<Self, String> {
        let handle = crate::runtime_handle()?;
        let client = Client::builder()
            .follow_redirects(false)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { handle, client })
    }

    async fn open_head(
        &self,
        request: &HttpRequest,
    ) -> Result<(Response<ResponseBody>, String, bool), HttpError> {
        if request.cancel.is_cancelled() {
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }

        let mut current_uri = validate_http_uri(&request.url)?;
        let mut method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| HttpError::new(HttpFailure::BadUrl, "invalid HTTP method"))?;
        let mut headers = filter_initial_headers(&request.headers);
        let mut body = if method == Method::GET || method == Method::HEAD {
            Vec::new()
        } else {
            request.body.clone()
        };

        let deadline = request
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms));
        let mut redirects = 0usize;

        loop {
            let wire_request = build_hop_request(&method, &current_uri, &headers, &body)?;
            let remaining = remaining_until(deadline)?;

            let mut call = self.client.new_call(wire_request).follow_redirects(false);
            if let Some(timeout) = remaining {
                call = call.call_timeout(timeout);
            }
            let call_handle = call.handle();

            let response = race_with_cancel_and_deadline(
                &request.cancel,
                deadline,
                || call_handle.cancel(),
                call.execute(),
            )
            .await?
            .map_err(map_wire_error)?;

            let status = response.status();
            if is_redirect_status(status)
                && let Some(location) = header_location(response.headers())
            {
                match request.redirect {
                    RedirectMode::Manual => {
                        let final_url = current_uri.to_string();
                        return Ok((response, final_url, redirects > 0));
                    }
                    RedirectMode::Error => {
                        return Err(HttpError::new(
                            HttpFailure::Network,
                            "redirect disallowed by redirect mode",
                        ));
                    }
                    RedirectMode::Follow => {
                        redirects += 1;
                        if redirects > MAX_REDIRECTS {
                            return Err(HttpError::new(HttpFailure::Network, "too many redirects"));
                        }
                        let next_uri = resolve_redirect_uri(&current_uri, &location)?;
                        validate_redirect_target(&current_uri, &next_uri)?;
                        if !same_origin(&current_uri, &next_uri) {
                            strip_cross_origin_headers(&mut headers);
                        }
                        apply_redirect_method(&status, &mut method, &mut headers, &mut body);
                        current_uri = next_uri;
                        continue;
                    }
                }
            }

            let final_url = current_uri.to_string();
            return Ok((response, final_url, redirects > 0));
        }
    }

    pub(crate) async fn execute_to_sink(
        &self,
        request: HttpRequest,
        mut sink: ResponseSink,
    ) -> Result<(), PublicError> {
        let cancel = request.cancel.clone();
        let deadline = request
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms));
        let limit = request.max_body_bytes.unwrap_or(HTTP_BODY_LIMIT);

        let (response, final_url, redirected) = match self.open_head(&request).await {
            Ok(head) => head,
            Err(err) => {
                return sink.complete(Err(err));
            }
        };

        let status = response.status().as_u16();
        let headers = collect_headers(response.headers());
        if let Err(err) = sink
            .send_head_async(status, &final_url, redirected, &headers)
            .await
        {
            return sink.complete(Err(err));
        }

        let (_, mut body) = response.into_parts();
        let mut received: usize = 0;

        loop {
            let next_frame =
                race_with_cancel_and_deadline(&cancel, deadline, || {}, body.frame()).await;
            match next_frame {
                Err(err) => {
                    return sink.complete(Err(err));
                }
                Ok(None) => {
                    return sink.complete(Ok(()));
                }
                Ok(Some(Err(err))) => {
                    return sink.complete(Err(map_wire_error(err)));
                }
                Ok(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        received = received.saturating_add(data.len());
                        if received > limit {
                            return sink.complete(Err(HttpError::new(
                                HttpFailure::TooLarge,
                                "response body exceeds limit",
                            )));
                        }
                        if let Err(err) = sink.send_chunk_async(data).await {
                            return sink.complete(Err(err));
                        }
                    }
                }
            }
        }
    }

    async fn stream_sync(
        &self,
        request: &HttpRequest,
        sink: &mut dyn FnMut(HttpPiece<'_>) -> bool,
    ) -> Result<(), HttpError> {
        let limit = request.max_body_bytes.unwrap_or(HTTP_BODY_LIMIT);
        let deadline = request
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms));
        let (response, final_url, redirected) = self.open_head(request).await?;
        let status = response.status().as_u16();
        let headers = collect_headers(response.headers());
        if !sink(HttpPiece::Head {
            status,
            url: &final_url,
            redirected,
            headers: &headers,
        }) {
            return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
        }

        let (_, mut body) = response.into_parts();
        let mut received: usize = 0;
        loop {
            if request.cancel.is_cancelled() {
                return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
            }
            let next_frame =
                race_with_cancel_and_deadline(&request.cancel, deadline, || {}, body.frame())
                    .await?;
            let Some(frame_result) = next_frame else {
                break;
            };
            let frame = frame_result.map_err(map_wire_error)?;
            let Some(data) = frame.data_ref() else {
                continue;
            };
            received = received.saturating_add(data.len());
            if received > limit {
                return Err(HttpError::new(
                    HttpFailure::TooLarge,
                    "response body exceeds limit",
                ));
            }
            if !sink(HttpPiece::Chunk(data, false)) {
                return Err(HttpError::new(HttpFailure::Canceled, "AbortError"));
            }
        }
        sink(HttpPiece::Chunk(&[], true));
        Ok(())
    }
}

impl HttpClient for OpenWireClient {
    fn perform(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut status = 0;
        let mut url = request.url.clone();
        let mut redirected = false;
        let mut headers = Vec::new();
        let mut body = Vec::new();
        self.stream(request, &mut |piece| {
            match piece {
                HttpPiece::Head {
                    status: s,
                    url: u,
                    redirected: r,
                    headers: h,
                } => {
                    status = s;
                    url = u.to_owned();
                    redirected = r;
                    headers = h.to_vec();
                }
                HttpPiece::Chunk(bytes, _) => body.extend_from_slice(bytes),
            }
            true
        })?;
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
        self.handle.block_on(self.stream_sync(request, sink))
    }
}

async fn race_with_cancel_and_deadline<F, T, C>(
    cancel: &CancelToken,
    deadline: Option<Instant>,
    on_abort: C,
    future: F,
) -> Result<T, HttpError>
where
    F: Future<Output = T>,
    C: FnOnce(),
{
    let future = std::pin::pin!(future);
    let cancelled = std::pin::pin!(cancel.cancelled());
    match deadline {
        Some(deadline) => {
            let sleep = std::pin::pin!(tokio::time::sleep_until(tokio::time::Instant::from_std(
                deadline
            )));
            match select(select(cancelled, sleep), future).await {
                Either::Left((Either::Left(((), _)), _)) => {
                    on_abort();
                    Err(HttpError::new(HttpFailure::Canceled, "AbortError"))
                }
                Either::Left((Either::Right(((), _)), _)) => {
                    on_abort();
                    Err(HttpError::new(HttpFailure::Timeout, "TimeoutError"))
                }
                Either::Right((output, _)) => Ok(output),
            }
        }
        None => match select(cancelled, future).await {
            Either::Left(((), _)) => {
                on_abort();
                Err(HttpError::new(HttpFailure::Canceled, "AbortError"))
            }
            Either::Right((output, _)) => Ok(output),
        },
    }
}

fn remaining_until(deadline: Option<Instant>) -> Result<Option<Duration>, HttpError> {
    let Some(deadline) = deadline else {
        return Ok(None);
    };
    let now = Instant::now();
    if now >= deadline {
        return Err(HttpError::new(HttpFailure::Timeout, "TimeoutError"));
    }
    Ok(Some(deadline.saturating_duration_since(now)))
}

fn validate_http_uri(raw: &str) -> Result<Uri, HttpError> {
    let without_fragment = raw.split('#').next().unwrap_or("");
    let uri = without_fragment
        .parse::<Uri>()
        .map_err(|_| HttpError::new(HttpFailure::BadUrl, "unsupported URL"))?;
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| HttpError::new(HttpFailure::BadUrl, "unsupported URL"))?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(HttpError::new(HttpFailure::BadUrl, "unsupported URL"));
    }
    if uri.authority().is_none_or(|auth| auth.host().is_empty()) {
        return Err(HttpError::new(HttpFailure::BadUrl, "unsupported URL"));
    }
    Ok(uri)
}

fn validate_redirect_target(current: &Uri, next: &Uri) -> Result<(), HttpError> {
    let next_scheme = next
        .scheme_str()
        .ok_or_else(|| HttpError::new(HttpFailure::BadUrl, "unsupported redirect URL"))?;
    if !next_scheme.eq_ignore_ascii_case("http") && !next_scheme.eq_ignore_ascii_case("https") {
        return Err(HttpError::new(
            HttpFailure::BadUrl,
            "unsupported redirect URL",
        ));
    }
    if current
        .scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"))
        && next_scheme.eq_ignore_ascii_case("http")
    {
        return Err(HttpError::new(
            HttpFailure::Network,
            "insecure HTTP redirect from HTTPS",
        ));
    }
    if next.authority().is_none_or(|auth| auth.host().is_empty()) {
        return Err(HttpError::new(
            HttpFailure::BadUrl,
            "unsupported redirect URL",
        ));
    }
    Ok(())
}

fn build_hop_request(
    method: &Method,
    uri: &Uri,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<Request<RequestBody>, HttpError> {
    let mut builder = Request::builder().method(method.clone()).uri(uri.clone());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| HttpError::new(HttpFailure::BadUrl, error.to_string()))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| HttpError::new(HttpFailure::BadUrl, error.to_string()))?;
        builder = builder.header(header_name, header_value);
    }
    let body = if *method == Method::GET || *method == Method::HEAD || body.is_empty() {
        RequestBody::empty()
    } else {
        RequestBody::from_bytes(Bytes::copy_from_slice(body))
    };
    builder
        .body(body)
        .map_err(|error| HttpError::new(HttpFailure::BadUrl, error.to_string()))
}

fn filter_initial_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| !hop_by_hop(name) && !name.eq_ignore_ascii_case("host"))
        .cloned()
        .collect()
}

fn collect_headers(headers: &http::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn header_location(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get(http::header::LOCATION)
        .and_then(|value| std::str::from_utf8(value.as_bytes()).ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn is_redirect_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn resolve_redirect_uri(base: &Uri, location: &str) -> Result<Uri, HttpError> {
    let location = location.split('#').next().unwrap_or("");
    if has_scheme_prefix(location) {
        return validate_http_uri(location);
    }
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base
        .authority()
        .ok_or_else(|| HttpError::new(HttpFailure::BadUrl, "missing base authority"))?
        .as_str();

    if location.starts_with("//") {
        return validate_http_uri(&format!("{scheme}:{location}"));
    }

    let path_and_query = if location.is_empty() {
        base.path_and_query()
            .map_or_else(|| "/".to_owned(), |pq| pq.as_str().to_owned())
    } else if location.starts_with('/') {
        normalize_path_and_query(location)
    } else if location.starts_with('?') {
        format!("{}{}", base.path(), location)
    } else {
        let base_path = base.path();
        let prefix = match base_path.rfind('/') {
            Some(pos) => &base_path[..=pos],
            None => "/",
        };
        normalize_path_and_query(&format!("{prefix}{location}"))
    };

    validate_http_uri(&format!("{scheme}://{authority}{path_and_query}"))
}

fn has_scheme_prefix(value: &str) -> bool {
    let cutoff = value.find(['/', '?', '#']).unwrap_or(value.len());
    let Some(colon) = value[..cutoff].find(':') else {
        return false;
    };
    let mut chars = value[..colon].chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn normalize_path_and_query(raw: &str) -> String {
    let (path, query) = match raw.find('?') {
        Some(idx) => (&raw[..idx], Some(&raw[idx..])),
        None => (raw, None),
    };
    let mut stack: Vec<&str> = Vec::new();
    let trailing_slash = path.ends_with('/');
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            part => stack.push(part),
        }
    }
    let mut out = String::from("/");
    out.push_str(&stack.join("/"));
    if trailing_slash && !stack.is_empty() && !out.ends_with('/') {
        out.push('/');
    }
    if let Some(query) = query {
        out.push_str(query);
    }
    out
}

fn same_origin(a: &Uri, b: &Uri) -> bool {
    let scheme_eq = a
        .scheme_str()
        .zip(b.scheme_str())
        .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y));
    let host_eq = a
        .host()
        .zip(b.host())
        .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y));
    scheme_eq && host_eq && default_port(a) == default_port(b)
}

fn default_port(uri: &Uri) -> Option<u16> {
    uri.port_u16().or_else(|| match uri.scheme_str() {
        Some(s) if s.eq_ignore_ascii_case("http") => Some(80),
        Some(s) if s.eq_ignore_ascii_case("https") => Some(443),
        _ => None,
    })
}

fn strip_cross_origin_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("authorization")
            && !name.eq_ignore_ascii_case("cookie")
            && !name.eq_ignore_ascii_case("proxy-authorization")
    });
}

fn apply_redirect_method(
    status: &StatusCode,
    method: &mut Method,
    headers: &mut Vec<(String, String)>,
    body: &mut Vec<u8>,
) {
    let drop_body = match status.as_u16() {
        303 => {
            if *method != Method::HEAD {
                *method = Method::GET;
            }
            true
        }
        301 | 302 if *method == Method::POST => {
            *method = Method::GET;
            true
        }
        _ => false,
    };
    if drop_body {
        body.clear();
        headers.retain(|(name, _)| {
            !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("content-type")
                && !name.eq_ignore_ascii_case("content-encoding")
                && !name.eq_ignore_ascii_case("content-language")
                && !name.eq_ignore_ascii_case("content-location")
        });
    }
}

fn hop_by_hop(name: &str) -> bool {
    name.eq_ignore_ascii_case("connection")
        || name.eq_ignore_ascii_case("keep-alive")
        || name.eq_ignore_ascii_case("proxy-connection")
        || name.eq_ignore_ascii_case("te")
        || name.eq_ignore_ascii_case("trailer")
        || name.eq_ignore_ascii_case("transfer-encoding")
        || name.eq_ignore_ascii_case("upgrade")
}

fn map_wire_error(error: WireError) -> HttpError {
    let kind = match error.kind() {
        WireErrorKind::Canceled => HttpFailure::Canceled,
        WireErrorKind::Timeout => HttpFailure::Timeout,
        WireErrorKind::InvalidRequest => HttpFailure::BadUrl,
        _ => HttpFailure::Network,
    };
    HttpError::new(kind, error.to_string())
}

#[cfg(test)]
mod redirect_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn get_request(url: String, redirect: RedirectMode) -> HttpRequest {
        HttpRequest {
            method: "GET".to_owned(),
            url,
            headers: vec![("Authorization".to_owned(), "Bearer secret".to_owned())],
            body: Vec::new(),
            cancel: CancelToken::new(),
            timeout_ms: Some(5_000),
            redirect,
            max_body_bytes: None,
        }
    }

    #[test]
    fn https_redirect_refuses_downgrade_to_http_or_non_http() {
        let current = Uri::from_static("https://example.test/start");
        assert!(matches!(
            validate_redirect_target(&current, &Uri::from_static("http://example.test/next")),
            Err(HttpError {
                kind: HttpFailure::Network,
                ..
            })
        ));
        assert!(matches!(
            validate_redirect_target(&current, &Uri::from_static("ftp://example.test/file")),
            Err(HttpError {
                kind: HttpFailure::BadUrl,
                ..
            })
        ));
        assert!(
            validate_redirect_target(&current, &Uri::from_static("https://example.test/next"))
                .is_ok()
        );
    }

    #[test]
    fn relative_location_and_redirect_modes_work_over_local_tcp() {
        let target_listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
        let target_port = target_listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let target_server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = target_listener.accept() else {
                return String::new();
            };
            let mut buf = [0u8; 2048];
            let read = stream.read(&mut buf).unwrap_or(0);
            let raw = String::from_utf8_lossy(&buf[..read]).into_owned();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone");
            raw
        });

        let redirect_listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("{e}"));
        let redirect_port = redirect_listener
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(0);
        let redirect_server = std::thread::spawn(move || {
            for _ in 0..3 {
                let Ok((mut stream, _)) = redirect_listener.accept() else {
                    break;
                };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target_port}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let client = OpenWireClient::new().unwrap_or_else(|e| panic!("{e}"));
        let url = format!("http://127.0.0.1:{redirect_port}/start");

        let manual = client
            .perform(&get_request(url.clone(), RedirectMode::Manual))
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(manual.status, 302);
        assert!(!manual.redirected);
        assert_eq!(manual.url, url);

        let err = client
            .perform(&get_request(url.clone(), RedirectMode::Error))
            .expect_err("redirect error mode must fail");
        assert_eq!(err.kind, HttpFailure::Network);

        let followed = client
            .perform(&get_request(url, RedirectMode::Follow))
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(followed.status, 200);
        assert!(followed.redirected);
        assert_eq!(
            followed.url,
            format!("http://127.0.0.1:{target_port}/final")
        );
        assert_eq!(followed.body, b"done");

        let seen_request = target_server
            .join()
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            !seen_request.contains("authorization:"),
            "cross-origin redirect leaked authorization header: {seen_request}"
        );
        let _ = redirect_server.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn request(url: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".to_owned(),
            url: url.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            cancel: CancelToken::new(),
            timeout_ms: None,
            redirect: RedirectMode::Follow,
            max_body_bytes: None,
        }
    }

    #[test]
    fn ftp_is_rejected_before_a_connection() {
        let client = OpenWireClient::new().unwrap_or_else(|error| panic!("{error}"));
        match client.perform(&request("ftp://example.test")) {
            Err(error) => assert_eq!(error.kind, HttpFailure::BadUrl),
            Ok(_) => panic!("ftp should fail"),
        }
    }

    #[test]
    fn http_get_reads_a_local_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
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
        let client = OpenWireClient::new().unwrap_or_else(|error| panic!("{error}"));
        let response = client
            .perform(&request(&format!("http://127.0.0.1:{port}/")))
            .unwrap_or_else(|error| panic!("{}", error.message));
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert!(!response.redirected);
        let _ = server.join();
    }

    #[test]
    fn streamed_pieces_join_into_the_full_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|error| panic!("{error}"));
        let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
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
        let client = OpenWireClient::new().unwrap_or_else(|error| panic!("{error}"));
        let mut head: Option<u16> = None;
        let mut body = Vec::new();
        let mut closed = false;
        let outcome = client.stream(
            &request(&format!("http://127.0.0.1:{port}/")),
            &mut |piece| {
                match piece {
                    HttpPiece::Head { status, .. } => head = Some(status),
                    HttpPiece::Chunk(bytes, done) => {
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
}
