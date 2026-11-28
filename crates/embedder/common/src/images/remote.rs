//! Bounded HTTPS reads for image assets.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::diagnostics::TelemetryHub;
use http::Request;
use http_body_util::BodyExt as _;
use impasto::network::{OpenWireHop, OpenWireObserver, RequestInfo, RequestTrace};
use openwire::{Client, RequestBody};
use url::Url;

use super::IMAGE_BODY_LIMIT;

const MAX_REDIRECTS: usize = 20;
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn fetch(source: &str, hub: Option<&Arc<TelemetryHub>>) -> Result<Vec<u8>, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            Client::builder()
                .follow_redirects(false)
                .event_listener_factory(OpenWireObserver)
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let url = Url::parse(source).map_err(|error| error.to_string())?;
    let mut trace = hub.map(|hub| {
        RequestTrace::new(
            Arc::clone(hub) as _,
            RequestInfo {
                initiator: "image",
                method: "GET",
                url: source,
                headers: &[],
                body: &[],
            },
        )
    });
    let result = crate::rt::handle()?.block_on(async {
        tokio::time::timeout(LOAD_TIMEOUT, read(client, url, trace.as_mut()))
            .await
            .map_err(|_| "HTTPS image load timed out".to_owned())?
    });
    if let Some(trace) = trace {
        trace.finish(result.as_ref().err().map(String::as_str), false);
    }
    result
}

async fn read(
    client: &Client,
    mut url: Url,
    mut trace: Option<&mut RequestTrace>,
) -> Result<Vec<u8>, String> {
    for index in 0..=MAX_REDIRECTS {
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return Err("image URL must use HTTPS without embedded credentials".into());
        }
        url.set_fragment(None);
        let mut request = Request::get(url.as_str())
            .body(RequestBody::empty())
            .map_err(|error| error.to_string())?;
        let hop = trace
            .as_ref()
            .map(|trace| OpenWireHop::install(&mut request, trace.hops()));
        let response = client
            .new_call(request)
            .follow_redirects(false)
            .execute()
            .await
            .map_err(|error| error.to_string())?;
        if let Some(hop) = &hop {
            hop.response(
                response.status().as_u16(),
                response
                    .headers()
                    .get(http::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            );
        }
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(location) = response.headers().get(http::header::LOCATION)
        {
            let next = location.to_str().map_err(|error| error.to_string())?;
            url = url.join(next).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(trace) = trace.as_mut() {
            let heads: Vec<_> = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.to_string(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            trace.head(response.status().as_u16(), url.as_str(), index != 0, &heads);
        }
        if !response.status().is_success() {
            return Err(format!("HTTP {} for {url}", response.status()));
        }
        let mut body = response.into_body();
        let mut image = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|error| error.to_string())?;
            let Some(data) = frame.data_ref() else {
                continue;
            };
            if image.len().saturating_add(data.len()) > IMAGE_BODY_LIMIT {
                return Err("image body exceeds limit".into());
            }
            if let Some(trace) = trace.as_mut() {
                trace.chunk(data);
            }
            image.extend_from_slice(data);
        }
        return Ok(image);
    }
    Err("too many image redirects".into())
}
