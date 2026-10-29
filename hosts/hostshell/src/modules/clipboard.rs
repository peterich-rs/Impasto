use std::sync::Arc;

use session::{HostMethod, HostModule, PluginValue};

use super::str_arg;

pub trait ClipboardHost: Send + Sync {
    fn read_clipboard(&self) -> String;
    fn write_clipboard(&self, text: &str);
}

pub fn module(host: Arc<dyn ClipboardHost>) -> HostModule {
    HostModule {
        name: "Clipboard",
        specifier: "@uifw/clipboard/host",
        methods: vec![
            HostMethod::sync("getString", {
                let host = Arc::clone(&host);
                move |_| Ok(PluginValue::Str(host.read_clipboard()))
            }),
            HostMethod::sync("setString", {
                let host = Arc::clone(&host);
                move |args| {
                    host.write_clipboard(&str_arg(args));
                    Ok(PluginValue::Null)
                }
            }),
        ],
    }
}
