//! Impasto application commands. Debug ESM for run, release bytecode for build.

mod analyze;
mod app;
mod build;
mod clean;
mod create;
mod ios;
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
        /// `macos`, `android`, an Android serial, `ios`, or `ios:<UDID>`.
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
        /// Keep watching the app and send source updates to the running iOS debug app.
        #[arg(long, conflicts_with = "release")]
        watch: bool,
    },
    /// Pack a release artifact. Defaults to release; `--debug` keeps ESM.
    Build {
        /// `bundle`, `apk`, `appbundle`, `aar`, `ios`, `ios-archive`, `ipa`, `ios-xcframework`, or `macos`.
        target: String,
        /// Build an iOS Simulator app without device signing.
        #[arg(long)]
        ios_simulator: bool,
        /// Xcode export options for a locally exported IPA.
        #[arg(long)]
        export_options: Option<PathBuf>,
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
    Test {
        /// Source file or trusted bytecode produced by this version of `impasto build`.
        bundle: Option<PathBuf>,
    },
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
            watch,
        } => finish(run::run(run::RunOptions {
            device,
            release,
            verbose,
            trace,
            watch,
        })),
        Command::Build {
            target,
            ios_simulator,
            export_options,
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
            ios_simulator,
            export_options.as_deref(),
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
