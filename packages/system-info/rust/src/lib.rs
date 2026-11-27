//! Host application, device and runtime information.

use impasto::os::{NativePlugin, NativePluginError};
use impasto::{
    ImpastoError, ImpastoType, PluginContext, PluginInitError, PluginRegistrar, Runtime,
};
use serde::Deserialize;
use tokio::sync::OnceCell;

#[derive(Clone, Copy, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub enum PlatformName {
    Ios,
    Macos,
    Android,
    Linux,
}

#[derive(Clone, Copy, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub enum InstallerStore {
    AppStore,
    TestFlight,
    GooglePlay,
    Amazon,
    Huawei,
    Xiaomi,
    Sideload,
    Simulator,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub app_name: String,
    pub package_name: String,
    pub version: String,
    pub build_number: String,
    pub installer_store: InstallerStore,
    pub is_debug: bool,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub platform: PlatformName,
    pub os_version: String,
    pub os_build: String,
    #[impasto(nullable)]
    pub sdk_int: Option<u32>,
    pub model: String,
    pub brand: String,
    pub manufacturer: String,
    pub architecture: String,
    pub is_physical_device: bool,
    pub processor_count: u32,
    pub total_memory_bytes: f64,
    pub locale: String,
    pub time_zone: String,
}

#[derive(Clone, Debug, Deserialize, ImpastoType)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub free_disk_bytes: f64,
    pub total_disk_bytes: f64,
    pub is_low_power_mode: bool,
}

#[derive(Debug)]
pub struct SystemInfoError(NativePluginError);

impl ImpastoError for SystemInfoError {
    fn scope(&self) -> &'static str {
        "system-info"
    }
    fn code(&self) -> &'static str {
        if self.0.code == "host-unavailable" {
            "host-unavailable"
        } else {
            "platform"
        }
    }
    fn message(&self) -> String {
        self.0.message.clone()
    }
}

struct Info {
    native: NativePlugin,
    app: OnceCell<AppInfo>,
    device: OnceCell<DeviceInfo>,
    data_path: std::path::PathBuf,
    runtime: Runtime,
}

impl Info {
    fn new(ctx: &PluginContext) -> Self {
        Self {
            native: NativePlugin::new(ctx, "@impasto/system-info"),
            app: OnceCell::new(),
            device: OnceCell::new(),
            data_path: ctx
                .paths()
                .cache_dir()
                .map_or_else(|_| std::env::temp_dir(), std::path::Path::to_owned),
            runtime: ctx.runtime().clone(),
        }
    }

    async fn app(&self) -> Result<AppInfo, SystemInfoError> {
        self.app
            .get_or_try_init(|| async {
                if self.native.has_channel() {
                    self.native
                        .request("appInfo", &())
                        .await
                        .map_err(SystemInfoError)
                } else {
                    Ok(fallback_app())
                }
            })
            .await
            .cloned()
    }

    async fn device(&self) -> Result<DeviceInfo, SystemInfoError> {
        self.device
            .get_or_try_init(|| async {
                if self.native.has_channel() {
                    self.native
                        .request("deviceInfo", &())
                        .await
                        .map_err(SystemInfoError)
                } else {
                    self.blocking(fallback_device).await
                }
            })
            .await
            .cloned()
    }

    async fn runtime(&self) -> Result<RuntimeInfo, SystemInfoError> {
        if self.native.has_channel() {
            return self
                .native
                .request("runtimeInfo", &())
                .await
                .map_err(SystemInfoError);
        }
        let path = self.data_path.clone();
        self.blocking(move || RuntimeInfo {
            free_disk_bytes: fs2::available_space(&path).unwrap_or_default() as f64,
            total_disk_bytes: fs2::total_space(&path).unwrap_or_default() as f64,
            is_low_power_mode: false,
        })
        .await
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, SystemInfoError> {
        if self.native.is_hosted() {
            self.runtime
                .spawn_blocking(f)
                .await
                .map_err(|error| SystemInfoError(NativePluginError::platform(error)))
        } else {
            Ok(f())
        }
    }
}

fn fallback_app() -> AppInfo {
    let name = std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem()?.to_str().map(str::to_owned))
        .unwrap_or_default();
    AppInfo {
        app_name: name.clone(),
        package_name: name,
        version: String::new(),
        build_number: String::new(),
        installer_store: InstallerStore::Unknown,
        is_debug: cfg!(debug_assertions),
    }
}

fn fallback_device() -> DeviceInfo {
    let platform = match std::env::consts::OS {
        "macos" => PlatformName::Macos,
        "ios" => PlatformName::Ios,
        "android" => PlatformName::Android,
        _ => PlatformName::Linux,
    };
    let read = |path: &str| {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let mem = read("/proc/meminfo");
    let kb = mem
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    DeviceInfo {
        platform,
        os_version: read("/proc/sys/kernel/osrelease"),
        os_build: read("/proc/sys/kernel/version"),
        sdk_int: None,
        model: read("/sys/devices/virtual/dmi/id/product_name"),
        brand: read("/sys/devices/virtual/dmi/id/sys_vendor"),
        manufacturer: read("/sys/devices/virtual/dmi/id/sys_vendor"),
        architecture: std::env::consts::ARCH.into(),
        is_physical_device: true,
        processor_count: std::thread::available_parallelism()
            .ok()
            .and_then(|value| u32::try_from(value.get()).ok())
            .unwrap_or(1),
        total_memory_bytes: kb as f64 * 1024.0,
        locale: std::env::var("LANG").unwrap_or_default(),
        time_zone: std::env::var("TZ").unwrap_or_else(|_| read("/etc/timezone")),
    }
}

/// Reads cached application metadata for the current session.
pub async fn app_info(ctx: &PluginContext) -> Result<AppInfo, SystemInfoError> {
    match ctx.extension::<Info>() {
        Some(info) => info.app().await,
        None => Info::new(ctx).app().await,
    }
}

/// Reads cached device information.
pub async fn device_info(ctx: &PluginContext) -> Result<DeviceInfo, SystemInfoError> {
    match ctx.extension::<Info>() {
        Some(info) => info.device().await,
        None => Info::new(ctx).device().await,
    }
}

/// Reads current disk and power state.
pub async fn runtime_info(ctx: &PluginContext) -> Result<RuntimeInfo, SystemInfoError> {
    match ctx.extension::<Info>() {
        Some(info) => info.runtime().await,
        None => Info::new(ctx).runtime().await,
    }
}

#[impasto::module(name = "SystemInfo", specifier = "@impasto/system-info/native", init = init)]
struct SystemInfoPlugin(std::sync::Arc<Info>);

impl SystemInfoPlugin {
    fn init(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        ctx.insert_extension(Info::new(ctx));
        ctx.extension()
            .map(Self)
            .ok_or_else(|| PluginInitError::new("system information context detached"))
    }
}

#[impasto::export]
impl SystemInfoPlugin {
    pub fn is_system_info_host_available(&self) -> bool {
        self.0.native.is_hosted()
    }
    pub async fn get_app_info(&self) -> Result<AppInfo, SystemInfoError> {
        self.0.app().await
    }
    pub async fn get_device_info(&self) -> Result<DeviceInfo, SystemInfoError> {
        self.0.device().await
    }
    pub async fn get_runtime_info(&self) -> Result<RuntimeInfo, SystemInfoError> {
        self.0.runtime().await
    }
}

/// Registers system information bindings.
pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(SystemInfoPlugin::plugin());
}
