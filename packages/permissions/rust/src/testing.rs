//! A programmable native channel for testing permission flows on a headless context.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use impasto::PluginContext;
use impasto::os::NativeChannel;

use super::{Permission, PermissionError, PermissionStatus, ServiceStatus, decode};

/// The outcome of the next permission prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Grant,
    GrantOnce,
    SelectPhotos,
    Deny,
    DoNotAskAgain,
}

#[derive(Debug, Default)]
struct State {
    statuses: BTreeMap<Permission, PermissionStatus>,
    services: BTreeMap<Permission, ServiceStatus>,
    rationales: BTreeMap<Permission, bool>,
    choices: BTreeMap<Permission, VecDeque<Decision>>,
    once: Vec<Permission>,
    settings_opened: u32,
}

/// Controls a simulated permission service for one test session.
#[derive(Clone, Debug, Default)]
pub struct MockPermissions(Arc<Mutex<State>>);

impl MockPermissions {
    /// Installs the simulation before registering plugins in a non-hosted context.
    pub fn install(&self, ctx: &PluginContext) -> Result<(), PermissionError> {
        if ctx.is_hosted() || ctx.extension::<NativeChannel>().is_some() {
            return Err(PermissionError::HostUnavailable(
                "permission mocks require an unbound test context".into(),
            ));
        }
        let state = Arc::clone(&self.0);
        let channel = Arc::new_cyclic(|reply: &Weak<NativeChannel>| {
            let reply = Weak::clone(reply);
            NativeChannel::new(move |id, operation, payload| {
                let Some(reply) = reply.upgrade() else {
                    return false;
                };
                let result = state
                    .lock()
                    .map_err(|error| error.to_string())
                    .and_then(|mut state| state.reply(operation, payload).map(Some));
                reply.complete(id, result)
            })
        });
        ctx.insert_extension((*channel).clone());
        ctx.insert_extension(channel);
        Ok(())
    }

    /// Sets the current permission state, service state and rationale.
    pub fn configure(
        &self,
        permission: Permission,
        status: PermissionStatus,
        service: ServiceStatus,
        rationale: bool,
    ) {
        if let Ok(mut state) = self.0.lock() {
            state.statuses.insert(permission, status);
            state.services.insert(permission, service);
            state.rationales.insert(permission, rationale);
            state.once.retain(|item| *item != permission);
        }
    }

    /// Adds the next simulated response for a permission.
    pub fn respond_with(&self, permission: Permission, decision: Decision) {
        if let Ok(mut state) = self.0.lock() {
            state
                .choices
                .entry(permission)
                .or_default()
                .push_back(decision);
        }
    }

    /// Expires every temporary grant.
    pub fn expire_one_time_grants(&self) {
        if let Ok(mut state) = self.0.lock() {
            for permission in std::mem::take(&mut state.once) {
                state.statuses.insert(permission, PermissionStatus::Denied);
            }
        }
    }

    /// Counts navigation to application or permission-specific settings.
    pub fn settings_opened(&self) -> u32 {
        self.0.lock().map_or(0, |state| state.settings_opened)
    }
}

fn wire(value: impl impasto::IntoPluginValue) -> Result<String, String> {
    match value.into_value()? {
        impasto::abi::PluginValue::Str(text) => Ok(text),
        _ => Err("invalid permission response".into()),
    }
}

impl State {
    fn reply(&mut self, operation: &str, payload: &str) -> Result<String, String> {
        match operation {
            "@impasto/permissions/settings" => {
                self.settings_opened = self.settings_opened.saturating_add(1);
                Ok("true".into())
            }
            "@impasto/permissions/limitedPicker" => Ok((self.statuses.get(&Permission::Photos)
                == Some(&PermissionStatus::Limited))
            .to_string()),
            "@impasto/permissions/request" => {
                let value: serde_json::Value =
                    serde_json::from_str(payload).map_err(|error| error.to_string())?;
                let names: Vec<String> = serde_json::from_value(value["names"].clone())
                    .map_err(|error| error.to_string())?;
                let states = names
                    .into_iter()
                    .map(|name| {
                        let name = decode(name).map_err(|error| error.to_string())?;
                        wire(self.request(name))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                serde_json::to_string(&states).map_err(|error| error.to_string())
            }
            operation => {
                let name = decode(payload.to_owned()).map_err(|error| error.to_string())?;
                match operation {
                    "@impasto/permissions/status" => wire(self.status(name)),
                    "@impasto/permissions/service" => wire(
                        self.services
                            .get(&name)
                            .copied()
                            .unwrap_or(ServiceStatus::NotApplicable),
                    ),
                    "@impasto/permissions/rationale" => Ok(self
                        .rationales
                        .get(&name)
                        .copied()
                        .unwrap_or(false)
                        .to_string()),
                    _ => Err(format!("unknown permission operation {operation}")),
                }
            }
        }
    }

    fn status(&self, permission: Permission) -> PermissionStatus {
        self.statuses
            .get(&permission)
            .copied()
            .unwrap_or(PermissionStatus::Denied)
    }

    fn request(&mut self, permission: Permission) -> PermissionStatus {
        let current = self.status(permission);
        if !matches!(
            current,
            PermissionStatus::Denied | PermissionStatus::Limited | PermissionStatus::Provisional
        ) {
            return current;
        }
        let Some(choice) = self
            .choices
            .get_mut(&permission)
            .and_then(VecDeque::pop_front)
        else {
            return current;
        };
        let next = match choice {
            Decision::Grant => PermissionStatus::Granted,
            Decision::GrantOnce => {
                self.once.push(permission);
                PermissionStatus::Granted
            }
            Decision::SelectPhotos => PermissionStatus::Limited,
            Decision::Deny => PermissionStatus::Denied,
            Decision::DoNotAskAgain => PermissionStatus::PermanentlyDenied,
        };
        self.statuses.insert(permission, next);
        self.rationales.insert(permission, choice == Decision::Deny);
        next
    }
}
