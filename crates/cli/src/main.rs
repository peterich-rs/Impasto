//! Impasto application commands. Debug ESM for run, release bytecode for build.

mod analyze;
mod app;
mod build;
mod clean;
mod create;
mod packages;
mod run;
mod test;
mod toolchain;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "impasto", about = "Build and run an Impasto app")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write app.toml, package.json, lib/main.tsx, and assets/.
    Create { name: String },
    /// Pack in debug and open a window. `-d` selects a target or a device.
    Run {
        /// Target name (`android`, `macos`) or a device id.
        #[arg(short = 'd')]
        device: Option<String>,
        /// Pack ESM and skip bytecode. This is the default for `run`.
        #[arg(long, conflicts_with = "release")]
        debug: bool,
        /// Pack release bytecode instead of the debug default.
        #[arg(long)]
        release: bool,
        /// Print the packer path and frame counters.
        #[arg(long)]
        verbose: bool,
        /// Print one frame's script time, run_updates count, and set_* count.
        #[arg(long)]
        trace: bool,
    },
    /// Pack a release artifact. Defaults to release; `--debug` keeps ESM.
    Build {
        /// `bundle`, `apk`, `ios`, or `macos`.
        target: String,
        #[arg(long, conflicts_with = "release")]
        debug: bool,
        #[arg(long)]
        release: bool,
        #[arg(long)]
        verbose: bool,
    },
    /// Type-check the app.
    Analyze,
    /// Run tests without opening a window.
    Test { bundle: Option<PathBuf> },
    /// Delete `build/` and pack output. Leaves `lib/`, `assets/`, and `app.toml`.
    Clean,
    /// Package-manager commands. Only `get` is implemented.
    Packages {
        #[command(subcommand)]
        command: PackagesCommand,
    },
}

#[derive(Subcommand)]
enum PackagesCommand {
    /// Run `pnpm install`, forwarding any extra arguments.
    Get {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Create { name } => finish(create::create(&name)),
        Command::Run {
            device,
            debug: _,
            release,
            verbose,
            trace,
        } => finish(run::run(run::RunOptions {
            device,
            release,
            verbose,
            trace,
        })),
        Command::Build {
            target,
            debug,
            release: _,
            verbose,
        } => finish(build::build(
            &target,
            build::BuildOptions {
                debug,
                verbose,
                trace: false,
            },
        )),
        Command::Analyze => finish(analyze::analyze()),
        Command::Test { bundle } => finish(test::test(bundle)),
        Command::Clean => finish(clean::clean()),
        Command::Packages { command } => match command {
            PackagesCommand::Get { args } => finish(packages::get(&args)),
        },
    }
}

fn finish(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("impasto: {message}");
            ExitCode::from(1)
        }
    }
}

/// Framework checkout when this binary was built from one. Used so `impasto run`
/// inside the checkout still opens the gallery.
fn framework_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    if root.join("apps/gallery-solid/build.mjs").is_file() {
        root.canonicalize().ok()
    } else {
        None
    }
}
