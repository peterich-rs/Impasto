use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use jni::objects::{JObjectArray, JString, JValue};
use jni::strings::JNIStr;
use jni::sys::jlong;
use session::{MethodExec, PluginResult, Settle};

use super::Bridge;

pub(super) struct PermissionRequest {
    id: jlong,
    names: Vec<String>,
    settle: Settle,
}

impl Bridge {
    fn permission_string(&self, method: &JNIStr, name: &str, default: &str) -> String {
        let mut status = default.to_owned();
        self.call(|env, view| {
            let name = env.new_string(name)?;
            let value = env
                .call_method(
                    view,
                    method,
                    jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                    &[JValue::Object(&name)],
                )?
                .l()?;
            status = env.cast_local::<JString>(value)?.try_to_string(env)?;
            Ok(())
        });
        status
    }

    pub(super) fn finish_permission_request(&self, id: jlong, result: Result<Vec<String>, String>) {
        let request = {
            let mut slot = self
                .permission_request
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if slot.as_ref().is_none_or(|pending| pending.id != id) {
                return;
            }
            slot.take()
        };
        if let Some(request) = request {
            request.settle.send(match result {
                Ok(statuses) if statuses.len() == request.names.len() => {
                    embedder::permission_results(request.names.into_iter().zip(statuses))
                }
                Ok(_) => PluginResult::Failed {
                    message: "invalid Android permission result".into(),
                },
                Err(message) => PluginResult::Failed { message },
            });
        }
    }

    pub(super) fn cancel_permission_request(&self) {
        let request = self
            .permission_request
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(request) = request {
            request.settle.send(PluginResult::Failed {
                message: "permission request cancelled with host".into(),
            });
        }
    }
}

impl embedder::PermissionsHost for Bridge {
    fn permission_status(&self, name: &str) -> String {
        self.permission_string(jni::jni_str!("permissionStatus"), name, "denied")
    }

    fn permission_rationale(&self, name: &str) -> bool {
        let mut show = false;
        self.call(|env, view| {
            let name = env.new_string(name)?;
            show = env
                .call_method(
                    view,
                    jni::jni_str!("permissionRationale"),
                    jni::jni_sig!("(Ljava/lang/String;)Z"),
                    &[JValue::Object(&name)],
                )?
                .z()?;
            Ok(())
        });
        show
    }

    fn service_status(&self, name: &str) -> String {
        self.permission_string(jni::jni_str!("permissionService"), name, "notApplicable")
    }

    fn open_app_settings(&self) -> bool {
        let mut opened = false;
        self.call(|env, view| {
            opened = env
                .call_method(
                    view,
                    jni::jni_str!("openAppSettings"),
                    jni::jni_sig!("()Z"),
                    &[],
                )?
                .z()?;
            Ok(())
        });
        opened
    }

    fn request_permissions_async(
        self: Arc<Self>,
        mut names: Vec<String>,
        settle: Settle,
        _: &dyn MethodExec,
    ) {
        static NEXT_REQUEST: AtomicI64 = AtomicI64::new(1);
        names.sort_unstable();
        names.dedup();
        if names.len() > 64 || names.iter().any(|name| name.len() > 128) {
            settle.send(PluginResult::Failed {
                message: "invalid permission request".into(),
            });
            return;
        }
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        {
            let mut slot = self
                .permission_request
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if slot.is_some() {
                drop(slot);
                settle.send(PluginResult::Failed {
                    message: "a permission request is already open".into(),
                });
                return;
            }
            *slot = Some(PermissionRequest {
                id,
                names: names.clone(),
                settle,
            });
        }
        let mut posted = false;
        self.call(|env, view| {
            let empty = env.new_string("")?;
            let array = JObjectArray::<JString>::new(env, names.len(), &empty)?;
            for (index, name) in names.iter().enumerate() {
                let name = env.new_string(name)?;
                array.set_element(env, index, &name)?;
            }
            posted = env
                .call_method(
                    view,
                    jni::jni_str!("requestPermissions"),
                    jni::jni_sig!("(J[Ljava/lang/String;)Z"),
                    &[JValue::Long(id), JValue::Object(array.as_ref())],
                )?
                .z()?;
            Ok(())
        });
        if !posted {
            self.finish_permission_request(
                id,
                Err("permission request could not reach the host".into()),
            );
        }
    }
}
