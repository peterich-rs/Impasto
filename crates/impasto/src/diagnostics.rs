//! Host diagnostics available to plugins in a window session.

use std::sync::Arc;

use crate::abi::PluginValue;

type Query = dyn Fn(&str, &[PluginValue]) -> Result<PluginValue, String> + Send + Sync;

/// Receives host diagnostics queries from an attached package.
#[derive(Clone)]
pub struct DiagnosticsSource(Arc<Query>);

impl DiagnosticsSource {
    #[doc(hidden)]
    pub fn new(
        query: impl Fn(&str, &[PluginValue]) -> Result<PluginValue, String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(query))
    }

    /// Reads a snapshot or invokes a diagnostics operation supported by this host.
    pub fn query(&self, operation: &str, args: &[PluginValue]) -> Result<PluginValue, String> {
        (self.0)(operation, args)
    }
}
