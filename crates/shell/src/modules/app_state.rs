use session::{EventType, Method, Module, ModuleExports, PluginValue, Shared, shared_read};

pub fn module(state: Shared<String>) -> Module {
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
            ],
            ..Default::default()
        },
        methods: vec![
            Method::sync("getCurrentState", move |_| {
                Ok(PluginValue::Str(shared_read(&state)))
            })
            .with_types("", "string"),
        ],
    }
}
