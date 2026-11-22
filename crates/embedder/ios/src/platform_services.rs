//! One-shot platform calls. Swift owns the controls and native completion handlers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use embedder::{LinkingHost, PermissionsHost, StorageHost};
use engine::session::{MethodExec, PluginResult, PluginValue, Settle};
use serde_json::{Value, json};

type RequestSink = dyn Fn(u64, &str) + Send + Sync;

const MAX_PLUGIN_CALLS: usize = 32;
const MAX_PLUGIN_JSON_BYTES: usize = 32 * 1024;
const MAX_PLUGIN_JSON_DEPTH: usize = 32;

pub(crate) struct PlatformServices {
    send_request: Box<RequestSink>,
    requests: Mutex<Requests>,
}

#[derive(Default)]
struct Requests {
    last_id: u64,
    closed: bool,
    pending: HashMap<u64, Pending>,
}

struct Pending {
    settle: Settle,
    reply: Reply,
}

enum Reply {
    Nothing,
    Document,
    Permission,
    Permissions(Vec<String>),
    Bool,
    NativePlugin,
}

impl PlatformServices {
    pub(crate) fn new(send_request: impl Fn(u64, &str) + Send + Sync + 'static) -> Self {
        Self {
            send_request: Box::new(send_request),
            requests: Mutex::new(Requests::default()),
        }
    }

    fn request(&self, operation: &'static str, argument: &str, names: &[String], pending: Pending) {
        let message = json!({ "operation": operation, "argument": argument, "names": names });
        self.send(message, pending);
    }

    pub(super) fn invoke_native_plugin(
        &self,
        identifier: &str,
        method: &str,
        payload: &str,
        settle: Settle,
    ) {
        if !valid_plugin_name(identifier) || !valid_plugin_name(method) {
            settle.send(failed("Native plugin identifier and method must be 1–128 bytes without control characters"));
            return;
        }
        if let Err(message) = validate_plugin_json(payload) {
            settle.send(failed(message));
            return;
        }
        self.send(
            json!({
                "operation": "nativePlugin",
                "argument": "",
                "names": [],
                "plugin": { "identifier": identifier, "method": method, "payload": payload }
            }),
            Pending {
                settle,
                reply: Reply::NativePlugin,
            },
        );
    }

    fn send(&self, message: Value, pending: Pending) {
        let mut requests = self.lock();
        if requests.closed {
            drop(requests);
            pending.settle.send(failed("iOS session is closed"));
            return;
        }
        if matches!(pending.reply, Reply::NativePlugin)
            && requests
                .pending
                .values()
                .filter(|pending| matches!(pending.reply, Reply::NativePlugin))
                .count()
                >= MAX_PLUGIN_CALLS
        {
            drop(requests);
            pending
                .settle
                .send(failed("Too many pending native plugin calls"));
            return;
        }
        let Some(id) = requests.last_id.checked_add(1) else {
            drop(requests);
            pending
                .settle
                .send(failed("iOS request identifiers are exhausted"));
            return;
        };
        requests.last_id = id;
        requests.pending.insert(id, pending);
        drop(requests);

        (self.send_request)(id, &message.to_string());
    }

    pub(crate) fn contains(&self, id: u64) -> bool {
        self.lock().pending.contains_key(&id)
    }

    pub(crate) fn complete(&self, id: u64, status: u32, payload: Option<String>) {
        let pending = self.lock().pending.remove(&id);
        if let Some(Pending { settle, reply }) = pending {
            settle.send(reply.decode(status, payload));
        }
    }

    pub(crate) fn cancel_native_plugins(&self) {
        let pending = self
            .lock()
            .pending
            .extract_if(|_, pending| matches!(pending.reply, Reply::NativePlugin))
            .map(|(_, pending)| pending)
            .collect::<Vec<_>>();
        cancel(pending);
    }

    pub(crate) fn cancel_pending(&self) {
        let pending = std::mem::take(&mut self.lock().pending);
        cancel(pending.into_values());
    }

    pub(crate) fn cancel_all(&self) {
        let pending = {
            let mut requests = self.lock();
            requests.closed = true;
            std::mem::take(&mut requests.pending)
        };
        cancel(pending.into_values());
    }

    fn lock(&self) -> MutexGuard<'_, Requests> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn cancel(pending: impl IntoIterator<Item = Pending>) {
    for request in pending {
        request
            .settle
            .send(failed("iOS presentation context was disconnected"));
    }
}

fn failed(message: impl Into<String>) -> PluginResult {
    PluginResult::Failed {
        message: message.into(),
    }
}

impl Reply {
    fn decode(self, status: u32, payload: Option<String>) -> PluginResult {
        if status == 2 {
            let payload = payload.filter(|text| {
                !matches!(self, Self::NativePlugin) || text.len() <= MAX_PLUGIN_JSON_BYTES
            });
            return failed(payload.unwrap_or_else(|| "iOS platform request failed".into()));
        }
        if status == 1 {
            return match self {
                Self::Document | Self::Nothing => PluginResult::Done(PluginValue::Null),
                Self::Bool => PluginResult::Done(PluginValue::Bool(false)),
                Self::Permission | Self::Permissions(_) | Self::NativePlugin => {
                    failed("iOS platform request was cancelled")
                }
            };
        }
        if status != 0 {
            return failed("invalid iOS platform result status");
        }
        let value = match self {
            Self::Nothing => PluginValue::Null,
            Self::Document => {
                return embedder::picked_document(match payload {
                    Some(name) => engine::session::document_relative(&name).map(Some),
                    None => Err("missing iOS document name".into()),
                });
            }
            Self::Bool => match payload.as_deref() {
                Some("true") => PluginValue::Bool(true),
                Some("false") => PluginValue::Bool(false),
                _ => return failed("invalid iOS platform boolean"),
            },
            Self::Permission => {
                let Some(status) = payload.filter(|status| is_permission_status(status)) else {
                    return failed("invalid iOS permission response");
                };
                PluginValue::Str(status)
            }
            Self::NativePlugin => {
                let Some(payload) = payload else {
                    return failed("Missing native plugin JSON response");
                };
                if let Err(message) = validate_plugin_json(&payload) {
                    return failed(message);
                }
                PluginValue::Str(payload)
            }
            Self::Permissions(names) => {
                let Some(payload) = payload else {
                    return failed("missing iOS permission response");
                };
                let Ok(statuses) = serde_json::from_str::<Vec<String>>(&payload) else {
                    return failed("invalid iOS permission response");
                };
                if names.len() != statuses.len()
                    || !statuses.iter().all(|status| is_permission_status(status))
                {
                    return failed("invalid iOS permission response");
                }
                return embedder::permission_results(names.into_iter().zip(statuses));
            }
        };
        PluginResult::Done(value)
    }
}

fn valid_plugin_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

fn validate_plugin_json(text: &str) -> Result<(), &'static str> {
    fn bounded_depth(value: &Value, depth: usize) -> bool {
        if depth > MAX_PLUGIN_JSON_DEPTH {
            return false;
        }
        match value {
            Value::Array(items) => items.iter().all(|item| bounded_depth(item, depth + 1)),
            Value::Object(items) => items.values().all(|item| bounded_depth(item, depth + 1)),
            _ => true,
        }
    }

    if text.len() > MAX_PLUGIN_JSON_BYTES {
        return Err("Native plugin JSON exceeds the 32 KiB limit");
    }
    let value: Value = serde_json::from_str(text).map_err(|_| "Invalid native plugin JSON")?;
    if !bounded_depth(&value, 0) {
        return Err("Native plugin JSON exceeds the nesting limit");
    }
    Ok(())
}

fn is_permission_status(status: &str) -> bool {
    matches!(
        status,
        "granted" | "denied" | "restricted" | "permanentlyDenied" | "limited"
    )
}

impl LinkingHost for PlatformServices {
    fn open_url(&self, _url: &str) -> Result<(), String> {
        Err("UIKit requires an asynchronous link request".into())
    }

    fn open_url_async(self: Arc<Self>, url: String, settle: Settle, _exec: &dyn MethodExec) {
        self.request(
            "openURL",
            &url,
            &[],
            Pending {
                settle,
                reply: Reply::Nothing,
            },
        );
    }

    fn share_text_async(self: Arc<Self>, text: String, settle: Settle, _exec: &dyn MethodExec) {
        self.request(
            "shareText",
            &text,
            &[],
            Pending {
                settle,
                reply: Reply::Nothing,
            },
        );
    }
}

pub(super) struct SandboxStorage {
    pub(super) root: String,
    pub(super) platform: Arc<PlatformServices>,
}

impl StorageHost for SandboxStorage {
    fn document_dir(&self) -> Result<String, String> {
        Ok(self.root.clone())
    }

    fn pick_document_async(self: Arc<Self>, settle: Settle, _exec: &dyn MethodExec) {
        self.platform.request(
            "pickDocument",
            "",
            &[],
            Pending {
                settle,
                reply: Reply::Document,
            },
        );
    }
}

impl PermissionsHost for PlatformServices {
    fn permission_status(&self, _name: &str) -> String {
        "restricted".into()
    }

    fn permission_status_async(
        self: Arc<Self>,
        name: String,
        settle: Settle,
        _exec: &dyn MethodExec,
    ) {
        self.request(
            "permissionStatus",
            &name,
            &[],
            Pending {
                settle,
                reply: Reply::Permission,
            },
        );
    }

    fn request_permissions_async(
        self: Arc<Self>,
        names: Vec<String>,
        settle: Settle,
        _exec: &dyn MethodExec,
    ) {
        self.request(
            "requestPermissions",
            "",
            &names,
            Pending {
                settle,
                reply: Reply::Permissions(names.clone()),
            },
        );
    }

    fn permission_rationale(&self, _name: &str) -> bool {
        false
    }

    fn service_status(&self, _name: &str) -> String {
        "notApplicable".into()
    }

    fn open_app_settings(&self) -> bool {
        false
    }

    fn open_app_settings_async(self: Arc<Self>, settle: Settle, _exec: &dyn MethodExec) {
        self.request(
            "openAppSettings",
            "",
            &[],
            Pending {
                settle,
                reply: Reply::Bool,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn cancelled_and_late_results_are_settled_once() -> Result<(), Box<dyn std::error::Error>> {
        let (send_id, recv_id) = mpsc::channel();
        let services = PlatformServices::new(move |id, _| {
            let _ = send_id.send(id);
        });
        let (send_result, recv_result) = mpsc::channel();

        let submit = || {
            let send_result = send_result.clone();
            let settle = Settle::new(move |result| {
                let _ = send_result.send(result);
            });
            services.request(
                "pickDocument",
                "",
                &[],
                Pending {
                    settle,
                    reply: Reply::Document,
                },
            );
        };
        submit();
        let first = recv_id.recv_timeout(Duration::from_secs(1))?;
        assert!(services.contains(first));
        services.cancel_pending();
        assert!(matches!(
            recv_result.recv_timeout(Duration::from_secs(1))?,
            PluginResult::Failed { .. }
        ));
        services.complete(first, 0, Some("stale.txt".into()));
        assert!(recv_result.try_recv().is_err());

        submit();
        let second = recv_id.recv_timeout(Duration::from_secs(1))?;
        assert_ne!(first, second);
        services.complete(second, 1, None);
        assert!(matches!(
            recv_result.recv_timeout(Duration::from_secs(1))?,
            PluginResult::Done(PluginValue::Null)
        ));
        services.cancel_all();
        submit();
        assert!(matches!(
            recv_result.recv_timeout(Duration::from_secs(1))?,
            PluginResult::Failed { .. }
        ));
        assert!(recv_id.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn native_plugins_bound_payloads_and_cancel_only_their_own_requests()
    -> Result<(), mpsc::RecvTimeoutError> {
        let (send_id, recv_id) = mpsc::channel();
        let services = PlatformServices::new(move |id, request| {
            let _ = send_id.send((id, request.to_owned()));
        });
        let (send_result, recv_result) = mpsc::channel();
        let settle = || {
            let send_result = send_result.clone();
            Settle::new(move |result| {
                let _ = send_result.send(result);
            })
        };
        let outcome = || recv_result.recv_timeout(Duration::from_secs(1));

        for bad_json in [
            "[]!".to_owned(),
            format!("\"{}\"", "a".repeat(MAX_PLUGIN_JSON_BYTES)),
            format!(
                "{}0{}",
                "[".repeat(MAX_PLUGIN_JSON_DEPTH + 1),
                "]".repeat(MAX_PLUGIN_JSON_DEPTH + 1)
            ),
        ] {
            services.invoke_native_plugin("echo", "mirror", &bad_json, settle());
            assert!(matches!(outcome()?, PluginResult::Failed { .. }));
            assert!(recv_id.try_recv().is_err());
        }

        services.invoke_native_plugin("echo", "mirror", "[null,42]", settle());
        let (reply, json) = recv_id.recv_timeout(Duration::from_secs(1))?;
        assert!(json.contains(r#""method":"mirror","payload":"[null,42]""#));
        services.complete(reply, 0, Some("true".into()));
        assert_eq!(
            outcome()?,
            PluginResult::Done(PluginValue::Str("true".into()))
        );

        services.invoke_native_plugin("echo", "mirror", "null", settle());
        let (bad_reply, _) = recv_id.recv_timeout(Duration::from_secs(1))?;
        services.complete(bad_reply, 0, Some("<not JSON>".into()));
        assert!(matches!(outcome()?, PluginResult::Failed { .. }));

        let mut pending = Vec::new();
        for _ in 0..MAX_PLUGIN_CALLS {
            services.invoke_native_plugin("echo", "mirror", "null", settle());
            pending.push(recv_id.recv_timeout(Duration::from_secs(1))?.0);
        }
        services.invoke_native_plugin("echo", "mirror", "null", settle());
        assert!(matches!(outcome()?, PluginResult::Failed { .. }));
        services.request(
            "pickDocument",
            "",
            &[],
            Pending {
                settle: settle(),
                reply: Reply::Document,
            },
        );
        let (document, _) = recv_id.recv_timeout(Duration::from_secs(1))?;
        services.cancel_native_plugins();
        for id in pending {
            assert!(matches!(outcome()?, PluginResult::Failed { .. }));
            services.complete(id, 0, Some("true".into()));
        }
        assert!(recv_result.try_recv().is_err());
        assert!(services.contains(document));
        services.complete(document, 1, None);
        assert_eq!(outcome()?, PluginResult::Done(PluginValue::Null));
        Ok(())
    }
}
