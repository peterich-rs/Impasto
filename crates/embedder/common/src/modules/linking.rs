use std::sync::Arc;

use session::{
    EventType, Method, MethodExec, Module, ModuleExports, PluginResult, PluginValue, Settle,
    Shared, shared_read,
};

use super::{done_null, str_arg};

pub trait LinkingHost: Send + Sync + 'static {
    fn open_url(&self, url: &str) -> Result<(), String>;
    fn open_url_async(self: Arc<Self>, url: String, settle: Settle, exec: &dyn MethodExec) {
        exec.main_thread(
            settle,
            Box::new(move || match self.open_url(&url) {
                Ok(()) => done_null(),
                Err(message) => PluginResult::Failed { message },
            }),
        );
    }

    fn share_text_async(self: Arc<Self>, text: String, settle: Settle, exec: &dyn MethodExec) {
        exec.main_thread(
            settle,
            Box::new(move || match self.share_text(&text) {
                Ok(()) => done_null(),
                Err(message) => PluginResult::Failed { message },
            }),
        );
    }

    fn share_text(&self, _text: &str) -> Result<(), String> {
        Err("sharing is unavailable on this host".into())
    }
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
                    Arc::clone(&host).open_url_async(str_arg(args), settle, exec)
                }
            })
            .with_types("url: string", "void"),
            Method::promise("shareText", {
                move |args, settle, exec| {
                    Arc::clone(&host).share_text_async(str_arg(args), settle, exec)
                }
            })
            .awaits_user()
            .with_types("text: string", "void"),
        ],
    }
}
