use std::sync::Arc;

use session::{Method, MethodExec, Module, ModuleExports, PluginResult, PluginValue, Settle};

use super::{str_arg, string_list};

pub trait PermissionsHost: Send + Sync {
    fn permission_status(&self, name: &str) -> String;
    fn permission_status_async(self: Arc<Self>, name: String, settle: Settle, exec: &dyn MethodExec)
    where
        Self: 'static,
    {
        exec.main_thread(
            settle,
            Box::new(move || PluginResult::Done(PluginValue::Str(self.permission_status(&name)))),
        );
    }

    fn request_permissions_async(
        self: Arc<Self>,
        names: Vec<String>,
        settle: Settle,
        exec: &dyn MethodExec,
    ) where
        Self: 'static,
    {
        exec.main_thread(
            settle,
            Box::new(move || {
                permission_results(names.into_iter().map(|name| {
                    let status = self.permission_status(&name);
                    (name, status)
                }))
            }),
        );
    }
    fn permission_rationale(&self, name: &str) -> bool;
    fn service_status(&self, name: &str) -> String;
    fn open_app_settings(&self) -> bool;
    fn open_app_settings_async(self: Arc<Self>, settle: Settle, exec: &dyn MethodExec)
    where
        Self: 'static,
    {
        exec.main_thread(
            settle,
            Box::new(move || PluginResult::Done(PluginValue::Bool(self.open_app_settings()))),
        );
    }
}

pub fn permission_results(rows: impl IntoIterator<Item = (String, String)>) -> PluginResult {
    PluginResult::Done(PluginValue::Map(
        rows.into_iter()
            .map(|(name, status)| (name, PluginValue::Str(status)))
            .collect(),
    ))
}

pub fn module(host: Arc<dyn PermissionsHost>) -> Module {
    Module {
        name: "Permissions",
        specifier: "@impasto/permissions",
        exports: ModuleExports {
            typescript: r#"export type PermissionStatus =
  | "granted"
  | "denied"
  | "restricted"
  | "permanentlyDenied"
  | "limited"
  | "provisional";

export type ServiceStatus = "enabled" | "disabled" | "notApplicable";"#,
            ..Default::default()
        },
        methods: vec![
            Method::promise("checkPermissionStatus", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    Arc::clone(&host).permission_status_async(str_arg(args), settle, exec)
                }
            })
            .with_types("permission: string", "PermissionStatus"),
            Method::promise("requestPermissions", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let names = string_list(args.first());
                    Arc::clone(&host).request_permissions_async(names, settle, exec);
                }
            })
            .awaits_user()
            .with_types("permissions: string[]", "Record<string, PermissionStatus>"),
            Method::promise("shouldShowRequestPermissionRationale", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let name = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Bool(host.permission_rationale(&name)))
                        }),
                    );
                }
            })
            .with_types("permission: string", "boolean"),
            Method::promise("checkServiceStatus", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let name = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || {
                            PluginResult::Done(PluginValue::Str(host.service_status(&name)))
                        }),
                    );
                }
            })
            .with_types("permission: string", "ServiceStatus"),
            Method::promise("openAppSettings", {
                let host = Arc::clone(&host);
                move |_args, settle, exec| Arc::clone(&host).open_app_settings_async(settle, exec)
            })
            .with_types("", "boolean"),
        ],
    }
}
