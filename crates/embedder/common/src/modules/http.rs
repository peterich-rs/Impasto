//! Native `@impasto/http` module built on [`engine::session::Method::stream_with_exec`].

use std::sync::Arc;

use engine::session::{CancelToken, Method, Module, ModuleExports, PluginValue};

use crate::http::{HttpClient, HttpError, HttpFailure, HttpRequest, RedirectMode, ResponseSink};

const HTTP_TYPES: &str = "\
export interface NativeHttpRequest {
  method: string;
  url: string;
  headers?: [string, string][];
  body?: Uint8Array | ArrayBuffer | string;
  timeoutMs?: number;
  redirect?: \"follow\" | \"error\" | \"manual\";
  maxBodyBytes?: number;
}

export type NativeHttpEvent =
  | {
      type: \"head\";
      status: number;
      url: string;
      redirected: boolean;
      headers: [string, string][];
    }
  | {
      type: \"chunk\";
      data: ArrayBuffer | Uint8Array;
    };";

pub fn http(client: Option<Arc<dyn HttpClient>>) -> Module {
    Module {
        name: "Http",
        specifier: "@impasto/http",
        exports: ModuleExports {
            object: true,
            typescript: HTTP_TYPES,
            ..Default::default()
        },
        methods: vec![
            Method::stream_with_exec("send", move |args, sink, exec| {
                let Some(client) = client.as_ref().map(Arc::clone) else {
                    sink.fail(
                        HttpError::new(HttpFailure::Network, "network is not linked")
                            .wire_message(),
                    );
                    return;
                };
                let request = match parse_http_request(args.first(), sink.cancellation()) {
                    Ok(request) => request,
                    Err(message) => {
                        sink.fail(HttpError::new(HttpFailure::BadUrl, message).wire_message());
                        return;
                    }
                };
                client.stream_to_sink(request, ResponseSink::new(sink), exec);
            })
            .with_types(
                "request: NativeHttpRequest, signal?: AbortSignal",
                "ReadableStream<NativeHttpEvent>",
            ),
        ],
    }
}

fn parse_http_request(
    arg: Option<&PluginValue>,
    cancel: CancelToken,
) -> Result<HttpRequest, String> {
    let Some(PluginValue::Map(entries)) = arg else {
        return Err("HTTP request options must be an object".to_owned());
    };

    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers = Vec::new();
    let mut body = Vec::new();
    let mut timeout_ms = None;
    let mut redirect = RedirectMode::Follow;
    let mut max_body_bytes = None;

    for (key, value) in entries {
        match key.as_str() {
            "method" => match value {
                PluginValue::Str(text) if !text.is_empty() => method = Some(text.clone()),
                _ => return Err("HTTP request method must be a non-empty string".to_owned()),
            },
            "url" => match value {
                PluginValue::Str(text) if !text.is_empty() => url = Some(text.clone()),
                _ => return Err("HTTP request url must be a non-empty string".to_owned()),
            },
            "headers" => match value {
                PluginValue::Null => {}
                PluginValue::List(items) => {
                    headers.reserve(items.len());
                    for item in items {
                        let PluginValue::List(pair) = item else {
                            return Err("HTTP header entry must be [name, value]".to_owned());
                        };
                        if pair.len() != 2 {
                            return Err("HTTP header entry must have 2 strings".to_owned());
                        }
                        let (PluginValue::Str(name), PluginValue::Str(val)) = (&pair[0], &pair[1])
                        else {
                            return Err("HTTP header name and value must be strings".to_owned());
                        };
                        headers.push((name.clone(), val.clone()));
                    }
                }
                _ => return Err("HTTP headers must be an array of [name, value] pairs".to_owned()),
            },
            "body" => match value {
                PluginValue::Null => {}
                PluginValue::Bytes(bytes) => body = bytes.clone(),
                PluginValue::Str(text) => body = text.as_bytes().to_vec(),
                _ => return Err("HTTP request body must be a string or bytes".to_owned()),
            },
            "timeoutMs" => {
                timeout_ms = parse_non_negative_u64(value, "timeoutMs")?;
            }
            "redirect" => match value {
                PluginValue::Null => {}
                PluginValue::Str(mode) => redirect = RedirectMode::parse(mode)?,
                _ => return Err("HTTP redirect must be a string".to_owned()),
            },
            "maxBodyBytes" => {
                max_body_bytes = parse_non_negative_usize(value, "maxBodyBytes")?;
            }
            _ => {}
        }
    }

    let url = url.ok_or_else(|| "missing HTTP request url".to_owned())?;
    Ok(HttpRequest {
        method: method.unwrap_or_else(|| "GET".to_owned()),
        url,
        headers,
        body,
        cancel,
        timeout_ms,
        redirect,
        max_body_bytes,
    })
}

fn parse_non_negative_u64(value: &PluginValue, field: &str) -> Result<Option<u64>, String> {
    match value {
        PluginValue::Null => Ok(None),
        PluginValue::F64(n) if n.is_finite() && *n >= 0.0 => {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked finite and non-negative above"
            )]
            let ms = *n as u64;
            Ok(Some(ms))
        }
        _ => Err(format!("HTTP {field} must be a non-negative number")),
    }
}

fn parse_non_negative_usize(value: &PluginValue, field: &str) -> Result<Option<usize>, String> {
    match value {
        PluginValue::Null => Ok(None),
        PluginValue::F64(n) if n.is_finite() && *n >= 0.0 => {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "checked finite and non-negative above"
            )]
            let limit = *n as usize;
            Ok(Some(limit))
        }
        _ => Err(format!("HTTP {field} must be a non-negative number")),
    }
}
