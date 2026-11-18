use std::sync::Arc;

use session::{
    EventType, Method, Module, ModuleExports, PluginResult, PluginValue, Shared, shared_read,
};

use super::{done_null, str_arg};

pub trait LinkingHost: Send + Sync {
    fn open_url(&self, url: &str) -> Result<(), String>;
}

pub fn module(host: Arc<dyn LinkingHost>, launch_url: Shared<Option<String>>) -> Module {
    Module {
        name: "Linking",
        specifier: "@impasto/linking",
        exports: ModuleExports {
            object: true,
            events: &[EventType {
                name: "url",
                payload: "{ url: string }",
            }],
            ..Default::default()
        },
        methods: vec![
            Method::sync("getInitialURL", move |_| {
                Ok(match shared_read(&launch_url) {
                    Some(url) => PluginValue::Str(url),
                    None => PluginValue::Null,
                })
            })
            .with_types("", "string | null"),
            Method::promise("openURL", {
                let host = Arc::clone(&host);
                move |args, settle, exec| {
                    let url = str_arg(args);
                    let host = Arc::clone(&host);
                    exec.main_thread(
                        settle,
                        Box::new(move || match host.open_url(&url) {
                            Ok(()) => done_null(),
                            Err(message) => PluginResult::Failed { message },
                        }),
                    );
                }
            })
            .with_types("url: string", "void"),
        ],
    }
}
