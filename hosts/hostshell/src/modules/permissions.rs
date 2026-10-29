use std::sync::Arc;

use session::{HostMethod, HostModule, PluginResult, PluginValue};

use super::{str_arg, string_list};

pub trait PermissionsHost: Send + Sync {
    fn permission_status(&self, name: &str) -> String;
    fn request_permissions(&self, names: &[String]) -> Vec<(String, String)>;
    fn permission_rationale(&self, name: &str) -> bool;
    fn service_status(&self, name: &str) -> String;
    fn open_app_settings(&self) -> bool;
}

pub fn module(host: Arc<dyn PermissionsHost>) -> HostModule {
    HostModule {
        name: "Permissions",
        specifier: "@uifw/permissions/host",
        methods: vec![
            HostMethod::promise("checkPermissionStatus", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let name = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.worker(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Str(host.permission_status(&name)))
                        }),
                    );
                }
            }),
            HostMethod::promise("requestPermissions", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let names = string_list(args.first());
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || {
                            let rows = host
                                .request_permissions(&names)
                                .into_iter()
                                .map(|(name, status)| (name, PluginValue::Str(status)))
                                .collect();
                            PluginResult::Done(PluginValue::Map(rows))
                        }),
                    );
                }
            }),
            HostMethod::promise("shouldShowRequestPermissionRationale", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let name = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.worker(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Bool(host.permission_rationale(&name)))
                        }),
                    );
                }
            }),
            HostMethod::promise("checkServiceStatus", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let name = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.worker(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Str(host.service_status(&name)))
                        }),
                    );
                }
            }),
            HostMethod::promise("openAppSettings", {
                let host = Arc::clone(&host);
                move |_args, settle, exec| {
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Bool(host.open_app_settings()))
                        }),
                    );
                }
            }),
        ],
    }
}
