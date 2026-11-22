use session::{Method, Module, ModuleExports, PluginResult, PluginValue, Settle};

pub fn module(invoke: impl Fn(&str, &str, &str, Settle) + Send + Sync + 'static) -> Module {
    Module {
        name: "NativePlugins",
        specifier: "@impasto/native-plugins",
        exports: ModuleExports::default(),
        methods: vec![
            Method::promise("invoke", move |args, settle, _| {
                let [
                    PluginValue::Str(identifier),
                    PluginValue::Str(method),
                    PluginValue::Str(payload),
                ] = args
                else {
                    settle.send(PluginResult::Failed {
                        message: "Native plugin expects an identifier, method and JSON payload"
                            .into(),
                    });
                    return;
                };
                invoke(identifier, method, payload, settle);
            })
            .with_types(
                "identifier: string, method: string, payload: string",
                "string",
            ),
        ],
    }
}
