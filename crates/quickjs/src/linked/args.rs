//! A host call prepared before argument application.

use session::{Invoke, PluginValue};

pub(super) struct PreparedCall {
    pub(super) invoke: Invoke,
    pub(super) token: u64,
    pub(super) args: Vec<PluginValue>,
}
