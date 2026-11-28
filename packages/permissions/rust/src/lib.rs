//! Asynchronous system permissions and service state for the current host.

use std::collections::BTreeMap;
use std::sync::Arc;

use impasto::abi::PluginValue;
use impasto::os::NativeChannel;
use impasto::{
    FromPluginValue, ImpastoError, ImpastoType, IntoPluginValue, PluginContext, PluginInitError,
    PluginRegistrar,
};

/// A capability whose access can be checked or requested.
#[derive(Clone, Copy, Debug, Eq, Hash, ImpastoType, Ord, PartialEq, PartialOrd)]
pub enum Permission {
    Camera,
    Microphone,
    Speech,
    Location,
    LocationWhenInUse,
    LocationAlways,
    Contacts,
    CalendarWriteOnly,
    CalendarFullAccess,
    Reminders,
    Storage,
    ManageExternalStorage,
    AccessMediaLocation,
    Photos,
    PhotosAddOnly,
    Videos,
    Audio,
    MediaLibrary,
    Notification,
    CriticalAlerts,
    Bluetooth,
    BluetoothConnect,
    BluetoothScan,
    BluetoothAdvertise,
    NearbyWifiDevices,
    Sensors,
    SensorsAlways,
    ActivityRecognition,
    Phone,
    Sms,
    ScheduleExactAlarm,
    AccessNotificationPolicy,
    SystemAlertWindow,
    RequestInstallPackages,
    IgnoreBatteryOptimizations,
    AppTrackingTransparency,
    Assistant,
    BackgroundRefresh,
    ScreenCapture,
    Accessibility,
    FullDiskAccess,
}

impl Permission {
    fn wire_name(self) -> Result<String, PermissionError> {
        match self.into_value().map_err(PermissionError::Platform)? {
            PluginValue::Str(name) => Ok(name),
            _ => Err(PermissionError::Platform(
                "invalid permission encoding".into(),
            )),
        }
    }
}

/// Authorization state reported by the current platform.
#[derive(Clone, Copy, Debug, Eq, ImpastoType, PartialEq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    Restricted,
    PermanentlyDenied,
    Limited,
    Provisional,
}

/// Availability of a permission's system-wide service.
#[derive(Clone, Copy, Debug, Eq, ImpastoType, PartialEq)]
pub enum ServiceStatus {
    Enabled,
    Disabled,
    NotApplicable,
}

/// Options for an Apple notification prompt. Android uses the standard notification prompt.
#[derive(Clone, Copy, Debug, Default, ImpastoType)]
pub struct NotificationPermissionOptions {
    pub alert: Option<bool>,
    pub badge: Option<bool>,
    pub sound: Option<bool>,
    pub provisional: Option<bool>,
    pub critical_alert: Option<bool>,
}

/// Failure to perform a permission operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PermissionError {
    #[error("{0}")]
    NotEnabled(String),
    #[error("{0}")]
    MissingConfig(String),
    #[error("{0}")]
    AlreadyRequesting(String),
    #[error("{0}")]
    HostUnavailable(String),
    #[error("{0}")]
    Cancelled(String),
    #[error("{0}")]
    Platform(String),
}

impl PermissionError {
    fn from_reply(message: String) -> Self {
        if let Ok(reply) = serde_json::from_str::<serde_json::Value>(&message)
            && let (Some(code), Some(message)) = (reply["code"].as_str(), reply["message"].as_str())
        {
            let message = message.to_owned();
            return match code {
                "not-enabled" => Self::NotEnabled(message),
                "missing-config" => Self::MissingConfig(message),
                "already-requesting" => Self::AlreadyRequesting(message),
                "host-unavailable" => Self::HostUnavailable(message),
                "cancelled" => Self::Cancelled(message),
                _ => Self::Platform(message),
            };
        }
        match message.as_str() {
            "the platform session is closed" | "the platform window detached" => {
                Self::Cancelled(message)
            }
            "the platform window is unavailable" => Self::HostUnavailable(message),
            _ => Self::Platform(message),
        }
    }
}

impl ImpastoError for PermissionError {
    fn scope(&self) -> &'static str {
        "permissions"
    }

    fn code(&self) -> &'static str {
        match self {
            Self::NotEnabled(_) => "not-enabled",
            Self::MissingConfig(_) => "missing-config",
            Self::AlreadyRequesting(_) => "already-requesting",
            Self::HostUnavailable(_) => "host-unavailable",
            Self::Cancelled(_) => "cancelled",
            Self::Platform(_) => "platform",
        }
    }

    fn message(&self) -> String {
        self.to_string()
    }
}

struct PermissionClient {
    channel: Option<Arc<NativeChannel>>,
    hosted: bool,
    #[cfg(target_os = "macos")]
    runtime: Option<impasto::Runtime>,
}

impl PermissionClient {
    async fn status(&self, permission: Permission) -> Result<PermissionStatus, PermissionError> {
        let name = permission.wire_name()?;
        if let Some(reply) = self.platform("status", &name).await? {
            return decode(reply);
        }
        #[cfg(target_os = "macos")]
        if let Some(runtime) = &self.runtime {
            return runtime
                .spawn_main(move || macos::status(&name))
                .await
                .map_err(|error| PermissionError::HostUnavailable(error.to_string()));
        }
        Ok(PermissionStatus::Denied)
    }

    async fn request(
        &self,
        permissions: &[Permission],
        options: Option<NotificationPermissionOptions>,
    ) -> Result<BTreeMap<Permission, PermissionStatus>, PermissionError> {
        if self.channel.is_none() {
            let mut statuses = BTreeMap::new();
            for &permission in permissions {
                statuses.insert(permission, self.status(permission).await?);
            }
            return Ok(statuses);
        }
        let names = permissions
            .iter()
            .map(|name| name.wire_name())
            .collect::<Result<Vec<_>, _>>()?;
        let options = options.map(|options| {
            serde_json::json!({
                "alert": options.alert,
                "badge": options.badge,
                "sound": options.sound,
                "provisional": options.provisional,
                "criticalAlert": options.critical_alert,
            })
        });
        let payload = serde_json::json!({ "names": names, "notification": options }).to_string();
        let reply = self
            .platform("request", &payload)
            .await?
            .ok_or_else(|| PermissionError::Platform("missing permission response".into()))?;
        let statuses: Vec<String> = serde_json::from_str(&reply)
            .map_err(|error| PermissionError::Platform(error.to_string()))?;
        if statuses.len() != permissions.len() {
            return Err(PermissionError::Platform(
                "invalid permission response length".into(),
            ));
        }
        permissions
            .iter()
            .copied()
            .zip(statuses)
            .map(|(name, status)| Ok((name, decode(status)?)))
            .collect()
    }

    async fn platform(
        &self,
        operation: &str,
        payload: &str,
    ) -> Result<Option<String>, PermissionError> {
        let Some(channel) = &self.channel else {
            return Ok(None);
        };
        let operation = format!("@impasto/permissions/{operation}");
        channel
            .request(&operation, payload)
            .await
            .map_err(PermissionError::from_reply)?
            .map(Some)
            .ok_or_else(|| PermissionError::Platform("missing permission response".into()))
    }

    async fn rationale(&self, permission: Permission) -> Result<bool, PermissionError> {
        self.flag("rationale", &permission.wire_name()?).await
    }

    async fn service(&self, permission: Permission) -> Result<ServiceStatus, PermissionError> {
        self.platform("service", &permission.wire_name()?)
            .await?
            .map_or(Ok(ServiceStatus::NotApplicable), decode)
    }

    async fn flag(&self, operation: &str, payload: &str) -> Result<bool, PermissionError> {
        self.platform(operation, payload)
            .await?
            .map_or(Ok(false), |value| {
                value.parse().map_err(|_| {
                    PermissionError::Platform(format!("invalid boolean response for {operation}"))
                })
            })
    }

    async fn settings(&self, permission: Option<Permission>) -> Result<bool, PermissionError> {
        #[cfg(target_os = "macos")]
        if self.channel.is_none()
            && let Some(runtime) = &self.runtime
        {
            return runtime
                .spawn_main(macos::settings)
                .await
                .map_err(|error| PermissionError::HostUnavailable(error.to_string()));
        }
        let name = permission
            .map(Permission::wire_name)
            .transpose()?
            .unwrap_or_default();
        self.flag("settings", &name).await
    }
}

fn decode<T: FromPluginValue>(value: String) -> Result<T, PermissionError> {
    T::from_value(PluginValue::Str(value)).map_err(PermissionError::Platform)
}

fn client(ctx: &PluginContext) -> Result<Arc<PermissionClient>, PermissionError> {
    if let Some(client) = ctx.extension::<PermissionClient>() {
        return Ok(client);
    }
    Err(PermissionError::HostUnavailable(
        "permissions is not initialized".into(),
    ))
}

/// Reads a permission's current authorization.
pub async fn check(
    ctx: &PluginContext,
    permission: Permission,
) -> Result<PermissionStatus, PermissionError> {
    client(ctx)?.status(permission).await
}

/// Reads the current state and requests access if it has not been granted.
pub async fn ensure(
    ctx: &PluginContext,
    permission: Permission,
) -> Result<PermissionStatus, PermissionError> {
    let client = client(ctx)?;
    let status = client.status(permission).await?;
    if status == PermissionStatus::Granted {
        return Ok(status);
    }
    client
        .request(&[permission], None)
        .await?
        .remove(&permission)
        .ok_or_else(|| PermissionError::Platform("missing permission response".into()))
}

/// Requests the capabilities in the given order and returns their resulting authorization.
pub async fn request(
    ctx: &PluginContext,
    permissions: &[Permission],
) -> Result<BTreeMap<Permission, PermissionStatus>, PermissionError> {
    client(ctx)?.request(permissions, None).await
}

/// Requests notifications through the same platform queue as all permission requests.
pub async fn request_notification(
    ctx: &PluginContext,
    options: NotificationPermissionOptions,
) -> Result<PermissionStatus, PermissionError> {
    client(ctx)?
        .request(&[Permission::Notification], Some(options))
        .await?
        .remove(&Permission::Notification)
        .ok_or_else(|| PermissionError::Platform("missing notification response".into()))
}

/// Reads the associated system-wide service switch.
pub async fn service_status(
    ctx: &PluginContext,
    permission: Permission,
) -> Result<ServiceStatus, PermissionError> {
    client(ctx)?.service(permission).await
}

/// Reports whether the platform recommends a rationale before the next prompt.
pub async fn should_show_rationale(
    ctx: &PluginContext,
    permission: Permission,
) -> Result<bool, PermissionError> {
    client(ctx)?.rationale(permission).await
}

/// Opens the application's system settings.
pub async fn open_app_settings(ctx: &PluginContext) -> Result<bool, PermissionError> {
    open_system_settings(ctx, None).await
}

/// Opens the permission's system settings page when available.
pub async fn open_system_settings(
    ctx: &PluginContext,
    permission: Option<Permission>,
) -> Result<bool, PermissionError> {
    client(ctx)?.settings(permission).await
}

/// Requests a permission by its original string name.
#[deprecated(note = "use ensure with a Permission")]
pub async fn ensure_named(
    ctx: &PluginContext,
    permission: &str,
) -> Result<PermissionStatus, PermissionError> {
    let permission = match permission {
        "calendar" => Permission::CalendarFullAccess,
        "photoLibrary" => Permission::Photos,
        name => decode(name.to_owned())?,
    };
    ensure(ctx, permission).await
}

#[impasto::module(name = "Permissions", specifier = "@impasto/permissions/native", init = init)]
struct PermissionsPlugin(Arc<PermissionClient>);

impl PermissionsPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let client = PermissionClient {
            channel: ctx.extension(),
            hosted: ctx.is_hosted(),
            #[cfg(target_os = "macos")]
            runtime: ctx.is_hosted().then(|| ctx.runtime().clone()),
        };
        ctx.insert_extension(client);
        ctx.extension()
            .map(Self)
            .ok_or_else(|| PluginInitError::new("permissions context is closed"))
    }
}

#[impasto::export]
impl PermissionsPlugin {
    pub fn is_permission_host_available(&self) -> bool {
        self.0.hosted
    }

    pub async fn check_permission_status(
        &self,
        permission: Permission,
    ) -> Result<PermissionStatus, PermissionError> {
        self.0.status(permission).await
    }

    #[impasto::method(awaits_user)]
    pub async fn request_permissions(
        &self,
        permissions: Vec<Permission>,
    ) -> Result<BTreeMap<String, PermissionStatus>, PermissionError> {
        self.0
            .request(&permissions, None)
            .await?
            .into_iter()
            .map(|(permission, status)| Ok((permission.wire_name()?, status)))
            .collect()
    }

    #[impasto::method(awaits_user)]
    pub async fn request_notification(
        &self,
        options: NotificationPermissionOptions,
    ) -> Result<PermissionStatus, PermissionError> {
        self.0
            .request(&[Permission::Notification], Some(options))
            .await?
            .remove(&Permission::Notification)
            .ok_or_else(|| PermissionError::Platform("missing notification response".into()))
    }

    pub async fn should_show_request_permission_rationale(
        &self,
        permission: Permission,
    ) -> Result<bool, PermissionError> {
        self.0.rationale(permission).await
    }

    pub async fn check_service_status(
        &self,
        permission: Permission,
    ) -> Result<ServiceStatus, PermissionError> {
        self.0.service(permission).await
    }

    pub async fn open_app_settings(&self) -> Result<bool, PermissionError> {
        self.0.settings(None).await
    }

    pub async fn open_system_settings(
        &self,
        permission: Option<Permission>,
    ) -> Result<bool, PermissionError> {
        self.0.settings(permission).await
    }

    #[impasto::method(awaits_user)]
    pub async fn present_limited_library_picker(&self) -> Result<bool, PermissionError> {
        self.0.flag("limitedPicker", "").await
    }
}

#[cfg(target_os = "macos")]
mod macos;

/// Registers the system permission bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(PermissionsPlugin::plugin());
}

#[cfg(any(test, feature = "test-util"))]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use testing::{Decision, MockPermissions};

    fn resolve<T>(future: impl Future<Output = T>) -> T {
        match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("the mock should reply synchronously"),
        }
    }

    #[test]
    fn wire_values_round_trip_and_failures_remain_structured() {
        for (permission, wire) in [
            (Permission::PhotosAddOnly, "photosAddOnly"),
            (Permission::Sms, "sms"),
            (Permission::CalendarFullAccess, "calendarFullAccess"),
        ] {
            assert_eq!(permission.wire_name().unwrap(), wire);
            assert_eq!(decode::<Permission>(wire.to_owned()).unwrap(), permission);
        }
        for code in [
            "not-enabled",
            "missing-config",
            "already-requesting",
            "host-unavailable",
            "cancelled",
            "platform",
        ] {
            let message = serde_json::json!({ "code": code, "message": "details" }).to_string();
            let error = PermissionError::from_reply(message);
            assert_eq!(error.code(), code);
            assert_eq!(error.message(), "details");
        }
        assert_eq!(
            PermissionError::from_reply("the platform window detached".into()).code(),
            "cancelled"
        );
    }

    #[test]
    fn headless_plugin_handles_temporary_access_rationale_and_settings_retry() {
        let ctx = PluginContext::default();
        let mock = MockPermissions::default();
        mock.install(&ctx).unwrap();
        let _plugin = PermissionsPlugin::init(&ctx).unwrap();
        mock.configure(
            Permission::Location,
            PermissionStatus::Denied,
            ServiceStatus::Enabled,
            false,
        );
        mock.respond_with(Permission::Location, Decision::GrantOnce);
        assert_eq!(
            resolve(ensure(&ctx, Permission::Location)).unwrap(),
            PermissionStatus::Granted
        );
        assert_eq!(
            resolve(service_status(&ctx, Permission::Location)).unwrap(),
            ServiceStatus::Enabled
        );
        mock.expire_one_time_grants();
        assert_eq!(
            resolve(check(&ctx, Permission::Location)).unwrap(),
            PermissionStatus::Denied
        );

        mock.respond_with(Permission::Photos, Decision::Deny);
        mock.respond_with(Permission::Photos, Decision::DoNotAskAgain);
        assert_eq!(
            resolve(ensure(&ctx, Permission::Photos)).unwrap(),
            PermissionStatus::Denied
        );
        assert!(resolve(should_show_rationale(&ctx, Permission::Photos)).unwrap());
        assert_eq!(
            resolve(ensure(&ctx, Permission::Photos)).unwrap(),
            PermissionStatus::PermanentlyDenied
        );
        assert!(resolve(open_system_settings(&ctx, Some(Permission::Photos))).unwrap());
        assert_eq!(mock.settings_opened(), 1);
        mock.configure(
            Permission::Photos,
            PermissionStatus::Limited,
            ServiceStatus::NotApplicable,
            false,
        );
        assert_eq!(
            resolve(check(&ctx, Permission::Photos)).unwrap(),
            PermissionStatus::Limited
        );
    }
}
