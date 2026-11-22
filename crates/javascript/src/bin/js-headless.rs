//! Standalone JavaScript host demonstrating compilation, events, and transport.

use std::sync::mpsc::{self, TrySendError};
use std::time::Duration;

use javascript::compiler;
use javascript::host::{
    EventType, HostEvent, IoEvent, IoJob, Method, Module, ModuleExports, ModuleRegistry,
    PluginResult, PluginValue,
};
use javascript::runtime::IoBindings;
use javascript::runtime::{ModuleCatalog, ModuleOutcome, Runtime};

const MAIN: &str = "example:main";

const SOURCE: &str = r#"
    import { Echo, copy, later } from "example:echo";
    if (globalThis.compiledValue !== 42) throw Error("compiled script");

    let state;
    const onChange = event => { state = event; };
    Echo.addEventListener("change", onChange);
    const data = { entries: [null, true, 42, "雪"] };
    if (JSON.stringify(copy(data)) !== JSON.stringify(data)) throw Error("sync host value");
    if (JSON.stringify(await later(data)) !== JSON.stringify(data)) throw Error("promise host value");
    if (state.name !== "ready") throw Error("event payload");
    Echo.removeEventListener("change", onChange);

    const url = new URL("payload", "https://example.invalid/data/");
    const body = new TextEncoder().encode("测试");
    const response = await fetch(url.href, { method: "POST", body });
    if (response.status !== 201 || await response.text() !== "written") throw Error("HTTP result");
    export const completed = true;
"#;

fn echo(args: &[PluginValue]) -> PluginValue {
    args.first().cloned().unwrap_or(PluginValue::Null)
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

    let (submit, jobs) = mpsc::sync_channel(1);
    let transport = IoBindings::routed(move |job| match submit.try_send(job) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => Err(Box::new(job)),
    });
    let mut runtime = Runtime::builder()
        .modules(catalog)
        .host(transport)
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .eval_compiled(&setup)
        .map_err(|error| error.to_string())?;
    let ticket = runtime
        .start_compiled_module(&bytes, MAIN)
        .map_err(|error| error.to_string())?;
    runtime
        .enqueue(IoEvent::Host(HostEvent {
            module: "Echo".to_owned(),
            event: "change".to_owned(),
            payload: PluginValue::Map(vec![("name".to_owned(), PluginValue::Str("ready".into()))]),
        }))
        .map_err(|error| format!("event admission failed: {error:?}"))?;
    runtime.drive(10).map_err(|error| error.to_string())?;

    let job = jobs.try_recv().map_err(|error| error.to_string())?;
    let IoJob::Routed { route, job, .. } = job else {
        return Err(format!("expected a routed HTTP job, received {job:?}"));
    };
    let IoJob::HttpStream(request) = job.as_ref() else {
        return Err(format!("expected an HTTP job, received {job:?}"));
    };
    if request.url != "https://example.invalid/data/payload"
        || request.method != "POST"
        || request.body != "测试".as_bytes()
    {
        return Err("HTTP request did not preserve URL, method, and UTF-8 bytes".into());
    }
    if jobs.try_recv().is_ok() || !runtime.take_jobs().is_empty() {
        return Err("HTTP request was submitted twice".into());
    }

    for completion in [
        IoEvent::HttpHead {
            token: request.token,
            status: 201,
            headers: vec![("Content-Type".into(), "text/plain".into())],
        },
        IoEvent::HttpChunk {
            token: request.token,
            bytes: b"written".to_vec(),
            done: true,
            error: None,
        },
    ] {
        let credit = route
            .reserve(completion.payload_bytes(), Duration::ZERO)
            .ok_or_else(|| "HTTP completion exceeded the queue limit".to_owned())?;
        runtime
            .enqueue_delivery(route.deliver(completion, Some(credit)))
            .map_err(|error| format!("HTTP delivery failed: {error:?}"))?;
    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn consumer_runs_without_a_ui_crate() -> Result<(), String> {
        super::main()
    }
}
