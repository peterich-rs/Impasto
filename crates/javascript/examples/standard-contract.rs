//! Bridge between the repository's TypeScript checker and a real QuickJS runtime.

use std::io::{self, Read};

use javascript::runtime::IoBindings;
use javascript::runtime::Runtime;
use javascript::standard::StandardProfile;
use rquickjs::CatchResultExt;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [name, action] = args.as_slice() else {
        return Err("usage: standard-contract <profile> <declarations|inspect>".into());
    };
    let profile = StandardProfile::default();
    let (io, profile) = match name.as_str() {
        "full" => (IoBindings::jobs(), profile),
        "http" => (IoBindings::jobs(), profile.with_transports(true, false)),
        "socket" => (
            IoBindings::disconnected().with_socket_jobs(),
            profile.with_transports(false, true),
        ),
        "offline" => (
            IoBindings::disconnected(),
            profile.with_transports(false, false),
        ),
        _ => return Err(format!("unknown profile {name:?}")),
    };

    match action.as_str() {
        "declarations" => print!("{}", profile.declarations()),
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
