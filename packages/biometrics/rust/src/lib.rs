//! Local user authentication with operating-system credential prompts.

use std::sync::atomic::{AtomicU64, Ordering};

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub enum Biometry {
    Face,
    Fingerprint,
    Iris,
    OpticId,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub enum SecurityLevel {
    Strong,
    Weak,
    None,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub is_available: bool,
    pub is_enrolled: bool,
    pub has_device_credential: bool,
    pub biometry_types: Vec<Biometry>,
    pub security_level: SecurityLevel,
}

#[derive(Clone, Debug, Serialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticateOptions {
    pub reason: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub cancel_label: Option<String>,
    pub fallback_label: Option<String>,
    pub allow_device_credential: Option<bool>,
    pub allow_watch: Option<bool>,
    pub confirmation_required: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "kebab-case")]
pub enum Failure {
    Cancelled,
    #[impasto(rename = "user-fallback")]
    UserFallback,
    Failed,
    Lockout,
    #[impasto(rename = "permanent-lockout")]
    PermanentLockout,
    #[impasto(rename = "not-enrolled")]
    NotEnrolled,
    #[impasto(rename = "passcode-not-set")]
    PasscodeNotSet,
    #[impasto(rename = "not-available")]
    NotAvailable,
    #[impasto(rename = "system-cancelled")]
    SystemCancelled,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
pub struct Authentication {
    pub authenticated: bool,
    pub reason: Option<Failure>,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct BiometricsError {
    code: &'static str,
    message: String,
}

impl ImpastoError for BiometricsError {
    fn scope(&self) -> &'static str {
        "biometrics"
    }
    fn code(&self) -> &'static str {
        self.code
    }
    fn message(&self) -> String {
        self.message.clone()
    }
}
impl From<NativePluginError> for BiometricsError {
    fn from(error: NativePluginError) -> Self {
        Self {
            code: match error.code.as_str() {
                "host-unavailable" => "host-unavailable",
                "invalid-argument" => "invalid-argument",
                "missing-config" => "missing-config",
                "busy" => "busy",
                _ => "platform",
            },
            message: error.message,
        }
    }
}
fn platform(ctx: &PluginContext) -> NativePlugin {
    NativePlugin::new(ctx, "@impasto/biometrics")
}

/// Reads the operating system's biometric availability and enrollment.
pub async fn get_capabilities(ctx: &PluginContext) -> Result<Capabilities, BiometricsError> {
    Ok(platform(ctx).request("capabilities", &()).await?)
}

/// Presents an operating-system authentication prompt.
pub async fn authenticate(
    ctx: &PluginContext,
    options: AuthenticateOptions,
) -> Result<Authentication, BiometricsError> {
    prompt(&platform(ctx), next_prompt(), options).await
}

fn next_prompt() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
#[derive(Serialize)]
struct Prompt {
    id: u64,
    #[serde(flatten)]
    options: AuthenticateOptions,
}

async fn prompt(
    platform: &NativePlugin,
    id: u64,
    options: AuthenticateOptions,
) -> Result<Authentication, BiometricsError> {
    if options.reason.trim().is_empty() {
        return Err(BiometricsError {
            code: "invalid-argument",
            message: "An authentication reason is required".into(),
        });
    }
    Ok(platform
        .request("authenticate", &Prompt { id, options })
        .await?)
}

#[impasto::module(name = "Biometrics", specifier = "@impasto/biometrics/native", init = init)]
struct Biometrics(NativePlugin);
impl Biometrics {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self(platform(ctx)))
    }
}
#[impasto::export]
impl Biometrics {
    pub fn is_biometrics_host_available(&self) -> bool {
        self.0.is_hosted()
    }
    pub fn next_prompt_id(&self) -> u64 {
        next_prompt()
    }
    pub async fn get_capabilities(&self) -> Result<Capabilities, BiometricsError> {
        Ok(self.0.request("capabilities", &()).await?)
    }
    #[impasto::method(awaits_user)]
    pub async fn authenticate(
        &self,
        id: u64,
        options: AuthenticateOptions,
    ) -> Result<Authentication, BiometricsError> {
        prompt(&self.0, id, options).await
    }
    pub async fn cancel(&self, id: u64) -> Result<(), BiometricsError> {
        Ok(self.0.request("cancel", &id).await?)
    }
}
/// Registers the biometric authentication bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(Biometrics::plugin());
}
