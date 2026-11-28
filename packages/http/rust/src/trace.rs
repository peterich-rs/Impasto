use std::sync::Arc;

use impasto::{
    DiagnosticsSink,
    network::{RequestInfo, RequestTrace},
};

use crate::wire::{HttpPiece, HttpRequest};

pub(crate) fn start(
    sink: Option<&Arc<dyn DiagnosticsSink>>,
    request: &mut HttpRequest,
    initiator: &'static str,
) -> Option<RequestTrace> {
    let sink = Arc::clone(sink?);
    let trace = RequestTrace::new(
        sink,
        RequestInfo {
            initiator,
            method: &request.method,
            url: &request.url,
            headers: &request.headers,
            body: &request.body,
        },
    );
    request.hops = Some(trace.hops());
    Some(trace)
}

pub(crate) fn piece(trace: &mut Option<RequestTrace>, piece: &HttpPiece<'_>) {
    if let Some(trace) = trace {
        match *piece {
            HttpPiece::Head {
                status,
                url,
                redirected,
                headers,
            } => trace.head(status, url, redirected, headers),
            HttpPiece::Chunk(bytes, _) => trace.chunk(bytes),
        }
    }
}

pub(crate) fn finish<T>(trace: Option<RequestTrace>, result: &Result<T, crate::wire::HttpError>) {
    if let Some(trace) = trace {
        let error = result.as_ref().err();
        trace.finish(
            error.map(|error| error.message.as_str()),
            error.is_some_and(|error| error.kind == crate::wire::HttpFailure::Canceled),
        );
    }
}
