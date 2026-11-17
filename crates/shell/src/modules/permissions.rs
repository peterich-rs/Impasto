use std::sync::Arc;

use session::{Method, Module, PluginResult, PluginValue};

use super::{str_arg, string_list};

pub trait PermissionsHost: Send + Sync {
    fn permission_status(&self, name: &str) -> String;
    fn request_permissions(&self, names: &[String]) -> Vec<(String, String)>;
    fn permission_rationale(&self, name: &str) -> bool;
    fn service_status(&self, name: &str) -> String;
    fn open_app_settings(&self) -> bool;
}

pub fn module(host: Arc<dyn PermissionsHost>) -> Module {
    Module {
        name: "Permissions",
        specifier: "@impasto/permissions",
        methods: vec![
            Method::promise("checkPermissionStatus", {
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
            Method::promise("requestPermissions", {
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
            Method::promise("shouldShowRequestPermissionRationale", {
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
            Method::promise("checkServiceStatus", {
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
            Method::promise("openAppSettings", {
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
