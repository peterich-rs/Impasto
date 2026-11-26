//! Asynchronous system permissions and service status.

use std::collections::BTreeMap;
use std::sync::Arc;

use impasto::abi::PluginValue;
use impasto::os::NativeChannel;
use impasto::{
    FromPluginValue, ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar,
};

/// Permission state reported by the current platform.
#[derive(Clone, Copy, Debug, Eq, ImpastoType, PartialEq)]
pub enum PermissionStatus {
    Granted,
    Denied,
    Restricted,
    PermanentlyDenied,
    Limited,
    Provisional,
}

#[derive(Clone, Copy, Debug, Eq, ImpastoType, PartialEq)]
enum ServiceStatus {
    Enabled,
    Disabled,
    NotApplicable,
}

/// An unavailable or failed platform permission operation.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PermissionError(String);

impl ImpastoError for PermissionError {
    fn scope(&self) -> &'static str {
        "permissions"
    }
    fn code(&self) -> &'static str {
        "platform"
    }
    fn message(&self) -> String {
        self.0.clone()
    }
}

struct PermissionClient {
    channel: Option<Arc<NativeChannel>>,
    #[cfg(target_os = "macos")]
    runtime: Option<impasto::Runtime>,
}

impl PermissionClient {
    async fn status(&self, permission: &str) -> Result<PermissionStatus, PermissionError> {
        if let Some(channel) = &self.channel {
            return parse_status(platform_request(channel, "status", permission).await?);
        }
        #[cfg(target_os = "macos")]
        if let Some(runtime) = &self.runtime {
            let permission = permission.to_owned();
            return runtime
                .spawn_main(move || macos::status(&permission))
                .await
                .map_err(|error| PermissionError(error.to_string()));
        }
        Ok(PermissionStatus::Denied)
    }

    async fn request(
        &self,
        permissions: Vec<String>,
    ) -> Result<BTreeMap<String, PermissionStatus>, PermissionError> {
        if let Some(channel) = &self.channel {
            let payload = serde_json::to_string(&permissions)
                .map_err(|error| PermissionError(error.to_string()))?;
            let reply = platform_request(channel, "request", &payload).await?;
            let statuses: Vec<String> =
                serde_json::from_str(&reply).map_err(|error| PermissionError(error.to_string()))?;
            if statuses.len() != permissions.len() {
                return Err(PermissionError("Invalid permission response length".into()));
            }
            return permissions
                .into_iter()
                .zip(statuses)
                .map(|(name, status)| Ok((name, parse_status(status)?)))
                .collect();
        }
        let mut statuses = BTreeMap::new();
        for permission in permissions {
            let status = self.status(&permission).await?;
            statuses.insert(permission, status);
        }
        Ok(statuses)
    }
}

async fn platform_request(
    channel: &NativeChannel,
    operation: &str,
    payload: &str,
) -> Result<String, PermissionError> {
    let operation = format!("@impasto/permissions/{operation}");
    channel
        .request(&operation, payload)
        .await
        .map_err(PermissionError)?
        .ok_or_else(|| PermissionError("Missing permission response".into()))
}

fn parse_status(status: String) -> Result<PermissionStatus, PermissionError> {
    PermissionStatus::from_value(PluginValue::Str(status)).map_err(PermissionError)
}

/// Requests a permission using the instance installed for this session.
pub async fn ensure(
    ctx: &PluginContext,
    permission: &str,
) -> Result<PermissionStatus, PermissionError> {
    let client = ctx
        .extension::<Arc<PermissionClient>>()
        .ok_or_else(|| PermissionError("Permissions is not initialized".into()))?;
    if client.status(permission).await? == PermissionStatus::Granted {
        return Ok(PermissionStatus::Granted);
    }
    client
        .request(vec![permission.into()])
        .await?
        .remove(permission)
        .ok_or_else(|| PermissionError("Missing permission response".into()))
}

#[impasto::module(name = "Permissions", specifier = "@impasto/permissions/native", init = init)]
struct PermissionsPlugin(Arc<PermissionClient>);

impl PermissionsPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        let client = Arc::new(PermissionClient {
            channel: ctx.extension(),
            #[cfg(target_os = "macos")]
            runtime: ctx.is_hosted().then(|| ctx.runtime().clone()),
        });
        ctx.insert_extension(Arc::clone(&client));
        Ok(Self(client))
    }
}

#[impasto::export]
impl PermissionsPlugin {
    pub async fn check_permission_status(
        &self,
        permission: String,
    ) -> Result<PermissionStatus, PermissionError> {
        self.0.status(&permission).await
    }

    #[impasto::method(awaits_user)]
    pub async fn request_permissions(
        &self,
        permissions: Vec<String>,
    ) -> Result<BTreeMap<String, PermissionStatus>, PermissionError> {
        self.0.request(permissions).await
    }

    pub async fn should_show_request_permission_rationale(
        &self,
        permission: String,
    ) -> Result<bool, PermissionError> {
        #[cfg(target_os = "android")]
        if let Some(channel) = &self.0.channel {
            return Ok(platform_request(channel, "rationale", &permission).await? == "true");
        }
        let _ = permission;
        Ok(false)
    }

    pub async fn check_service_status(
        &self,
        permission: String,
    ) -> Result<ServiceStatus, PermissionError> {
        #[cfg(target_os = "android")]
        if let Some(channel) = &self.0.channel {
            let status = platform_request(channel, "service", &permission).await?;
            return ServiceStatus::from_value(PluginValue::Str(status)).map_err(PermissionError);
        }
        let _ = permission;
        Ok(ServiceStatus::NotApplicable)
    }

    pub async fn open_app_settings(&self) -> Result<bool, PermissionError> {
        if let Some(channel) = &self.0.channel {
            return Ok(platform_request(channel, "settings", "").await? == "true");
        }
        #[cfg(target_os = "macos")]
        if let Some(runtime) = &self.0.runtime {
            return runtime
                .spawn_main(macos::settings)
                .await
                .map_err(|error| PermissionError(error.to_string()));
        }
        Ok(false)
    }
}

#[cfg(target_os = "macos")]
mod macos;

/// Registers the system permission bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(PermissionsPlugin::plugin());
}
