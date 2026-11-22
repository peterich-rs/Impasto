use engine::session::{EventType, Method, Module, ModuleExports, PluginValue, Shared, shared_read};

use crate::{Restoration, WindowSession};

pub fn module(state: Shared<String>) -> Module {
    with_restoration(state, None, WindowSession::default())
}

pub(crate) fn with_restoration(
    state: Shared<String>,
    restoration: Option<Restoration>,
    window_session: WindowSession,
) -> Module {
    let read = restoration.clone();
    let window_read = window_session.clone();
    Module {
        name: "AppState",
        specifier: "@impasto/app-state",
        exports: ModuleExports {
            events: &[
                EventType {
                    name: "change",
                    payload: "string",
                },
                EventType {
                    name: "focus",
                    payload: "\"true\" | \"false\"",
                },
                EventType {
                    name: "windowState",
                    payload: "string",
                },
            ],
            ..Default::default()
        },
        methods: vec![
            Method::sync("getCurrentState", move |_| {
                Ok(PluginValue::Str(shared_read(&state)))
            })
            .with_types("", "string"),
            Method::sync("getRestorationState", move |args| {
                let key = super::str_arg(args);
                Ok(read
                    .as_ref()
                    .and_then(|store| store.get(&key))
                    .map_or(PluginValue::Null, PluginValue::Str))
            })
            .with_types("key: string", "string | null"),
            Method::sync("setRestorationState", move |args| {
                if let Some(store) = &restoration {
                    let key = super::str_arg(args);
                    let value = match args.get(1) {
                        Some(PluginValue::Str(text)) => Some(text.as_str()),
                        Some(PluginValue::Null) => None,
                        _ => return Err("restoration state must be a string or null".into()),
                    };
                    store.set(&key, value)?;
                }
                Ok(PluginValue::Null)
            })
            .with_types("key: string, value: string | null", "void"),
            Method::sync("getWindowState", move |args| {
                Ok(window_read
                    .get(&super::str_arg(args))
                    .map_or(PluginValue::Null, PluginValue::Str))
            })
            .with_types("key: string", "string | null"),
            Method::sync("setWindowState", move |args| {
                let key = super::str_arg(args);
                let value = match args.get(1) {
                    Some(PluginValue::Str(text)) => Some(text.as_str()),
                    Some(PluginValue::Null) => None,
                    _ => return Err("window state must be a string or null".into()),
                };
                window_session.set(&key, value)?;
                Ok(PluginValue::Null)
            })
            .with_types("key: string, value: string | null", "void"),
        ],
    }
}
