use session::{HostMethod, HostModule, PluginValue, Shared, shared_read};

pub fn module(state: Shared<String>) -> HostModule {
    HostModule {
        name: "AppState",
        specifier: "@impasto/app-state/host",
        methods: vec![HostMethod::sync("getCurrentState", move |_| {
            Ok(PluginValue::Str(shared_read(&state)))
        })],
    }
}
