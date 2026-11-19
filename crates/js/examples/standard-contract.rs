//! Bridge between the repository's TypeScript checker and a real QuickJS runtime.

use std::io::{self, Read};

use js::host::IoBindings;
use js::{Runtime, StandardProfile};
use rquickjs::CatchResultExt;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [name, action] = args.as_slice() else {
        return Err("usage: standard-contract <profile> <declarations|inspect>".into());
    };
    let portable = StandardProfile::portable();
    let (profile, io, expected) = match name.as_str() {
        "legacy" => (
            StandardProfile::legacy(),
            IoBindings::disconnected(),
            StandardProfile::legacy(),
        ),
        "portable" => (portable, IoBindings::jobs(), portable),
        "portable-http" => {
            let http_only = portable.with_transports(true, false);
            (http_only, IoBindings::jobs(), http_only)
        }
        "portable-socket" => (
            portable,
            IoBindings::disconnected().with_socket_jobs(),
            portable.with_transports(false, true),
        ),
        "portable-offline" => (
            portable,
            IoBindings::disconnected(),
            portable.with_transports(false, false),
        ),
        _ => return Err(format!("unknown profile {name:?}")),
    };

    match action.as_str() {
        "declarations" => print!("{}", expected.declarations()),
        "inspect" => {
            let mut script = String::new();
            io::stdin()
                .read_to_string(&mut script)
                .map_err(|error| error.to_string())?;
            let baseline = Runtime::builder()
                .build_unstarted()
                .map_err(|error| error.to_string())?;
            let keys = evaluate(&baseline, "JSON.stringify(Reflect.ownKeys(globalThis))")?;
            drop(baseline);
            let runtime = Runtime::builder()
                .host(io)
                .standard(profile)
                .build()
                .map_err(|error| error.to_string())?;
            let report = evaluate(
                &runtime,
                &format!("(function (baseline) {{\n{script}\n}})({keys})"),
            )?;
            println!("{report}");
        }
        _ => return Err(format!("unknown action {action:?}")),
    }
    Ok(())
}

fn evaluate(runtime: &Runtime, source: &str) -> Result<String, String> {
    let extension = runtime.extension();
    let _budget = extension.budget("standard contract", 0);
    extension.with(|ctx| {
        ctx.eval::<String, _>(source)
            .catch(&ctx)
            .map_err(|error| error.to_string())
    })
}
