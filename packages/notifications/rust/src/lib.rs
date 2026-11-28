//! Local notification authorization, attachment validation, and native dispatch.

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    CancelToken, ImpastoError, PluginContext, PluginInitError, PluginRegistrar, TypedSink,
};
use impasto_file_system::{Scope, resolve_path};
use impasto_permissions::{Permission, PermissionStatus};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug)]
struct NotificationError(&'static str, String);
impl NotificationError {
    fn new(code: &'static str, message: &str) -> Self {
        Self(code, message.to_owned())
    }
    fn invalid() -> Self {
        Self::new("invalid-argument", "Invalid notification options")
    }
}
impl ImpastoError for NotificationError {
    fn scope(&self) -> &'static str {
        "notifications"
    }
    fn code(&self) -> &'static str {
        self.0
    }
    fn message(&self) -> String {
        self.1.clone()
    }
}
impl From<NativePluginError> for NotificationError {
    fn from(error: NativePluginError) -> Self {
        let code = match error.code.as_str() {
            "permission-denied" => "permission-denied",
            "exact-alarm-denied" => "exact-alarm-denied",
            "invalid-argument" => "invalid-argument",
            "outside-scope" => "outside-scope",
            "file-not-found" => "file-not-found",
            "host-unavailable" => "host-unavailable",
            _ => "platform",
        };
        Self(code, error.message)
    }
}

#[derive(Deserialize)]
struct Attachment {
    path: String,
    scope: Option<Scope>,
    id: Option<String>,
}

fn identifier(value: &Value) -> Result<&str, NotificationError> {
    value
        .as_str()
        .filter(|text| {
            !text.trim().is_empty() && text.len() <= 256 && !text.chars().any(char::is_control)
        })
        .ok_or_else(NotificationError::invalid)
}

fn validate_trigger(trigger: &Value) -> Result<(), NotificationError> {
    let valid = match trigger.get("kind").and_then(Value::as_str) {
        Some("date") => trigger
            .get("atMs")
            .and_then(Value::as_i64)
            .is_some_and(|value| (1..=8_640_000_000_000_000).contains(&value)),
        Some("interval") => trigger
            .get("everySeconds")
            .and_then(Value::as_f64)
            .is_some_and(|value| {
                let repeats = trigger
                    .get("repeats")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                ((if repeats { 60.0 } else { 1.0 })..=31_536_000.0).contains(&value)
            }),
        Some("calendar") => {
            let fields = [
                ("year", 1, 9999),
                ("month", 1, 12),
                ("day", 1, 31),
                ("weekday", 1, 7),
                ("hour", 0, 23),
                ("minute", 0, 59),
                ("second", 0, 59),
            ];
            fields
                .iter()
                .any(|(field, _, _)| trigger.get(field).is_some())
                && fields.iter().all(|(field, low, high)| {
                    trigger.get(field).is_none_or(|value| {
                        value
                            .as_i64()
                            .is_some_and(|value| (*low..=*high).contains(&value))
                    })
                })
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(NotificationError::invalid())
    }
}

fn data(input: &Value) -> Result<Value, NotificationError> {
    let object = input.as_object().ok_or_else(NotificationError::invalid)?;
    if ["title", "body"]
        .iter()
        .any(|field| !object.get(*field).is_some_and(Value::is_string))
    {
        return Err(NotificationError::invalid());
    }
    identifier(&input["id"])?;
    if input.get("data").is_some_and(|data| {
        !data
            .as_object()
            .is_some_and(|object| object.values().all(Value::is_string))
    }) {
        return Err(NotificationError::invalid());
    }
    Ok(input.clone())
}

fn attachments(ctx: &PluginContext, input: Value) -> Result<Value, NotificationError> {
    let mut request = data(&input)?;
    if let Some(value) = request.get("attachments") {
        let files = serde_json::from_value::<Vec<Attachment>>(value.clone())
            .map_err(|_| NotificationError::invalid())?;
        if files.len() > 16 {
            return Err(NotificationError::invalid());
        }
        let files = files
            .into_iter()
            .map(|item| {
                let path = resolve_path(ctx, item.scope.unwrap_or_default(), &item.path).map_err(
                    |_| {
                        NotificationError::new(
                            "outside-scope",
                            "Notification attachment is outside its scope",
                        )
                    },
                )?;
                if !path.is_file() {
                    return Err(NotificationError::new(
                        "file-not-found",
                        "Notification attachment is unavailable",
                    ));
                }
                Ok(json!({"path": path, "id": item.id}))
            })
            .collect::<Result<Vec<_>, NotificationError>>()?;
        request["attachments"] = files.into();
    }
    Ok(request)
}

async fn permission(
    ctx: &PluginContext,
    capability: Permission,
    ask: bool,
) -> Result<PermissionStatus, NotificationError> {
    let result = if ask {
        impasto_permissions::ensure(ctx, capability).await
    } else {
        impasto_permissions::check(ctx, capability).await
    };
    result.map_err(|error| NotificationError("platform", error.to_string()))
}

#[impasto::module(name = "Notifications", specifier = "@impasto/notifications/native", init = init)]
struct Notifications {
    ctx: PluginContext,
    native: NativePlugin,
}

impl Notifications {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            ctx: ctx.clone(),
            native: NativePlugin::new(ctx, "@impasto/notifications"),
        })
    }
    async fn submit(
        &self,
        request: String,
        trigger: Option<Value>,
    ) -> Result<(), NotificationError> {
        if !self.native.has_channel() {
            return Err(NotificationError::new(
                "host-unavailable",
                "Notifications require the native host",
            ));
        }
        if let Some(trigger) = trigger.as_ref() {
            validate_trigger(trigger)?;
        }
        let request: Value =
            serde_json::from_str(&request).map_err(|_| NotificationError::invalid())?;
        data(&request)?;
        let status = permission(&self.ctx, Permission::Notification, true).await?;
        if !matches!(
            status,
            PermissionStatus::Granted | PermissionStatus::Limited | PermissionStatus::Provisional
        ) {
            return Err(NotificationError::new(
                "permission-denied",
                "Notification authorization is required",
            ));
        }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if request["interruptionLevel"].as_str() == Some("critical")
            && permission(&self.ctx, Permission::CriticalAlerts, false).await?
                != PermissionStatus::Granted
        {
            return Err(NotificationError::new(
                "permission-denied",
                "Critical alert authorization is required",
            ));
        }
        #[cfg(target_os = "android")]
        if trigger.as_ref().and_then(|value| value["exact"].as_bool()) == Some(true) {
            let status = permission(&self.ctx, Permission::ScheduleExactAlarm, false).await?;
            if !matches!(status, PermissionStatus::Granted) {
                return Err(NotificationError::new(
                    "exact-alarm-denied",
                    "Exact alarm authorization is required",
                ));
            }
        }
        let ctx = self.ctx.clone();
        let request = self
            .ctx
            .runtime()
            .spawn_blocking(move || attachments(&ctx, request))
            .await
            .map_err(|_| {
                NotificationError::new("host-unavailable", "Attachment validation is unavailable")
            })??;
        Ok(self
            .native
            .request("submit", &json!({"request": request, "trigger": trigger}))
            .await?)
    }
}

#[impasto::export]
impl Notifications {
    pub fn is_notifications_host_available(&self) -> bool {
        self.native.is_hosted()
    }
    #[impasto::method(awaits_user)]
    pub async fn show(&self, request: String) -> Result<(), NotificationError> {
        self.submit(request, None).await
    }
    #[impasto::method(awaits_user)]
    pub async fn schedule(
        &self,
        request: String,
        trigger: String,
    ) -> Result<(), NotificationError> {
        let trigger = serde_json::from_str(&trigger).map_err(|_| NotificationError::invalid())?;
        self.submit(request, Some(trigger)).await
    }
    pub async fn invoke(
        &self,
        operation: String,
        json: String,
    ) -> Result<String, NotificationError> {
        if !matches!(
            operation.as_str(),
            "cancel"
                | "cancelAll"
                | "getPending"
                | "dismissDelivered"
                | "createChannel"
                | "deleteChannel"
                | "listChannels"
                | "registerCategories"
                | "setBadgeCount"
                | "getBadgeCount"
                | "setForegroundPresentation"
                | "getInitialResponse"
        ) {
            return Err(NotificationError::invalid());
        }
        let payload: Value =
            serde_json::from_str(&json).map_err(|_| NotificationError::invalid())?;
        if matches!(operation.as_str(), "cancel" | "deleteChannel") {
            identifier(&payload)?;
        }
        let reply: Value = self.native.request(&operation, &payload).await?;
        Ok(reply.to_string())
    }
    pub async fn check_permission(&self) -> Result<PermissionStatus, NotificationError> {
        permission(&self.ctx, Permission::Notification, false).await
    }
    #[impasto::method(awaits_user)]
    pub async fn request_permission(
        &self,
        options: String,
    ) -> Result<PermissionStatus, NotificationError> {
        let options: Value =
            serde_json::from_str(&options).map_err(|_| NotificationError::invalid())?;
        let option = |key: &str| options.get(key).and_then(Value::as_bool);
        let options = impasto_permissions::NotificationPermissionOptions {
            alert: option("alert"),
            badge: option("badge"),
            sound: option("sound"),
            provisional: option("provisional"),
            critical_alert: option("criticalAlert"),
        };
        impasto_permissions::request_notification(&self.ctx, options)
            .await
            .map_err(|error| NotificationError("platform", error.to_string()))
    }
    pub async fn watch_responses(
        &self,
        sink: TypedSink<String>,
        cancel: CancelToken,
    ) -> Result<(), NotificationError> {
        self.native
            .watch_changes("response", sink, cancel)
            .await
            .map_err(Into::into)
    }
    pub async fn watch_foreground(
        &self,
        sink: TypedSink<String>,
        cancel: CancelToken,
    ) -> Result<(), NotificationError> {
        self.native
            .watch_changes("foreground", sink, cancel)
            .await
            .map_err(Into::into)
    }
}

/// Registers notification policies and local presentation bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Notifications::plugin());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_repeating_intervals_and_calendar_ranges() {
        assert!(
            validate_trigger(&json!({"kind":"interval","everySeconds":59,"repeats":true})).is_err()
        );
        assert!(
            validate_trigger(&json!({"kind":"interval","everySeconds":60,"repeats":true})).is_ok()
        );
        assert!(validate_trigger(&json!({"kind":"calendar","month":13})).is_err());
    }
}
