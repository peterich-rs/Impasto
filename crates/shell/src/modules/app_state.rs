use session::{Method, Module, PluginValue, Shared, shared_read};

pub fn module(state: Shared<String>) -> Module {
    Module {
        name: "AppState",
        specifier: "@impasto/app-state",
        methods: vec![Method::sync("getCurrentState", move |_| {
            Ok(PluginValue::Str(shared_read(&state)))
        })],
    }
}
