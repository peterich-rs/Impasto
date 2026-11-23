//! Standalone JavaScript host demonstrating compilation, events, resources, and streams.

use javascript::compiler;
use javascript::host::{
    CancelToken, EventType, HostEvent, Method, Module, ModuleExports, ModuleRegistry,
    NativeResource, PluginResult, PluginValue,
};
use javascript::runtime::{ModuleCatalog, ModuleOutcome, Runtime};

const MAIN: &str = "example:main";

const SOURCE: &str = r#"
    import { Echo, copy, later, open, chunks } from "example:echo";
    if (globalThis.compiledValue !== 42) throw Error("compiled script");

    let state;
    const onChange = event => { state = event; };
    Echo.addEventListener("change", onChange);
    const data = { entries: [null, true, 42, "雪"] };
    if (JSON.stringify(copy(data)) !== JSON.stringify(data)) throw Error("sync host value");
    if (JSON.stringify(await later(data)) !== JSON.stringify(data)) throw Error("promise host value");
    if (state.name !== "ready") throw Error("event payload");
    Echo.removeEventListener("change", onChange);

    const handle = open();
    if (handle.call("ping", "ok") !== "pong:ok") throw Error("resource method");
    handle.close();

    const url = new URL("payload", "https://example.invalid/data/");
    if (url.pathname !== "/data/payload") throw Error("URL pathname");

    const reader = chunks(new TextEncoder().encode("测试")).getReader();
    const first = await reader.read();
    const second = await reader.read();
    if (first.done || new TextDecoder().decode(first.value) !== "测试" || !second.done) {
        throw Error("native stream result");
    }
    export const completed = true;
"#;

fn echo(args: &[PluginValue]) -> PluginValue {
    args.first().cloned().unwrap_or(PluginValue::Null)
}

struct EchoHandle;

impl NativeResource for EchoHandle {
    fn call(
        &mut self,
        method: &str,
        args: &[PluginValue],
        _cancel: &CancelToken,
    ) -> Result<PluginValue, String> {
        match method {
            "ping" => match args.first() {
                Some(PluginValue::Str(text)) => Ok(PluginValue::Str(format!("pong:{text}"))),
                _ => Ok(PluginValue::Str("pong".into())),
            },
            _ => Err(format!("unknown method {method:?}")),
        }
    }
}

fn main() -> Result<(), String> {
    let registry = ModuleRegistry::try_new(vec![Module {
        name: "Echo",
        specifier: "example:echo",
        methods: vec![
            Method::sync("copy", |args| Ok(echo(args))).with_types("value: unknown", "unknown"),
            Method::promise("later", |args, settle, exec| {
                exec.now(settle, PluginResult::Done(echo(args)));
            })
            .with_types("value: unknown", "unknown"),
            Method::resource("open", |_| Ok(Box::new(EchoHandle))).with_types(
                "",
                "{ call(method: \"ping\", label?: string): string; close(): void }",
            ),
            Method::stream_with_exec("chunks", |args, mut sink, exec| {
                let bytes = match args.into_iter().next() {
                    Some(PluginValue::Bytes(bytes)) => bytes,
                    _ => Vec::new(),
                };
                exec.worker(
                    javascript::host::Settle::new(|_| {}),
                    Box::new(move || {
                        let _ = sink.send(
                            PluginValue::Bytes(bytes),
                            std::time::Duration::from_millis(100),
                        );
                        sink.finish();
                        PluginResult::Done(PluginValue::Null)
                    }),
                );
            })
            .with_types("payload: Uint8Array", "Uint8Array"),
        ],
        exports: ModuleExports {
            object: true,
            events: &[EventType {
                name: "change",
                payload: "{ name: string }",
            }],
            ..ModuleExports::default()
        },
    }])
    .map_err(|error| error.to_string())?;
    let mut catalog = ModuleCatalog::new(registry).map_err(|error| error.to_string())?;
    catalog
        .add_source(MAIN, SOURCE)
        .map_err(|error| error.to_string())?;
    let bytes =
        compiler::compile(SOURCE, MAIN, true, &catalog).map_err(|error| error.to_string())?;
    if !bytes.as_bytes().starts_with(b"IQJS") {
        return Err("missing bytecode envelope".into());
    }
    let setup = compiler::compile(
        "globalThis.compiledValue = 40 + 2;",
        "example:setup",
        false,
        &catalog,
    )
    .map_err(|error| error.to_string())?;

    let mut runtime = Runtime::builder()
        .modules(catalog)
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .eval_compiled(&setup)
        .map_err(|error| error.to_string())?;
    let ticket = runtime
        .start_compiled_module(&bytes, MAIN)
        .map_err(|error| error.to_string())?;
    runtime
        .enqueue(host_change_event())
        .map_err(|error| format!("event admission failed: {error:?}"))?;
    runtime.drive(10).map_err(|error| error.to_string())?;
    runtime.drive(11).map_err(|error| error.to_string())?;

    match runtime
        .take_module_result(ticket)
        .map_err(|error| error.to_string())?
    {
        Some(ModuleOutcome::Fulfilled) => {}
        Some(ModuleOutcome::Rejected(error)) => return Err(error.to_string()),
        None => return Err("module evaluation is still pending".into()),
    }
    let counts = runtime.counts();
    if [
        counts.promises,
        counts.requests,
        counts.tickets,
        counts.listeners,
        counts.queued_events,
        counts.queued_bytes,
    ]
    .iter()
    .any(|count| *count != 0)
    {
        return Err(format!("host resources are still live: {counts:?}"));
    }
    runtime.shutdown();
    Ok(())
}

fn host_change_event() -> javascript::host::IoEvent {
    javascript::host::IoEvent::Host(HostEvent {
        module: "Echo".to_owned(),
        event: "change".to_owned(),
        payload: PluginValue::Map(vec![("name".to_owned(), PluginValue::Str("ready".into()))]),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn consumer_runs_without_a_ui_crate() -> Result<(), String> {
        super::main()
    }
}
