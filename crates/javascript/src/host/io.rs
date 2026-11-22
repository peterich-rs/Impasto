//! Work that leaves the owning thread and comes back as an [`IoEvent`].

use super::{PluginResult, PluginValue};

#[path = "io_credit.rs"]
mod credit;
pub use credit::{Credit, CreditPool, CreditUsage, Delivery, RequestKey, Route, RuntimeKey};
pub(super) use credit::{register_waker, wake_all};

/// One owned IO completion. The host chooses the thread that delivers it.
pub enum IoEvent {
    NativeStream { id: u64, event: super::StreamEvent },
    Host(HostEvent),
    Plugin { token: u64, result: PluginResult },
}

/// Owned module event. Module identity is resolved by the receiving catalog.
pub struct HostEvent {
    pub module: String,
    pub event: String,
    pub payload: PluginValue,
}

pub const TERMINAL_PAYLOAD_LIMIT: usize = 1024;

pub const MAX_SAFE_ID: u64 = (1 << 53) - 1;

impl IoEvent {
    pub fn id(&self) -> Option<u64> {
        match self {
            Self::Plugin { token, .. } => Some(*token),
            Self::NativeStream { id, .. } => Some(*id),
            Self::Host(_) => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        match self {
            Self::Plugin { .. } => true,
            Self::NativeStream { event, .. } => !matches!(event, super::StreamEvent::Item(_)),
            Self::Host(_) => false,
        }
    }

    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::Plugin { result, .. } => match result {
                PluginResult::Done(value) => value.payload_bytes(),
                PluginResult::Failed { message } => message.len(),
            },
            Self::NativeStream { event, .. } => match event {
                super::StreamEvent::Item(value) => value.payload_bytes(),
                super::StreamEvent::Finished => 0,
                super::StreamEvent::Failed(message) => message.len(),
            },
            Self::Host(event) => event
                .module
                .len()
                .saturating_add(event.event.len())
                .saturating_add(event.payload.payload_bytes()),
        }
    }
}
