//! Impasto application commands. Debug ESM for run, release bytecode for build.

mod analyze;
mod android_watch;
mod app;
mod build;
mod clean;
mod create;
mod dependencies;
mod devtools;
mod distribute;
mod ios;
mod local_registry;
mod packages;
mod plugins;
mod run;
mod symbols;
mod sync;
mod test;
mod toolchain;
mod typescript;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

const GALLERY_DIR: &str = "examples/gallery-solid";

#[derive(Parser)]
#[command(name = "impasto", about = "Build and run an Impasto app")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create an app with editable Android, iOS, and macOS projects.
    Create {
        name: String,
        #[arg(long)]
        with_rust: bool,
    },
    /// Add an application-local Rust crate.
    InitRust,
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
        /// Enable component hot updates for the running debug app.
        #[arg(long, conflicts_with = "release")]
        watch: bool,
    },
    /// Pack a release artifact. Defaults to release; `--debug` keeps ESM.
    Build {
        /// `bundle`, `apk`, `appbundle`, `aar`, `ios`, `ios-archive`, `ipa`, `ios-xcframework`, or `macos`.
        target: String,
        /// Select the native plugin platform for build bundle.
        #[arg(long)]
        platform: Option<String>,
        /// Check a bundle update against the native manifest from an installed app.
        #[arg(long)]
        installed_manifest: Option<PathBuf>,
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
    /// Link a standalone TSX or JavaScript entry to an ES module.
    Pack {
        entry: PathBuf,
        output: PathBuf,
        #[arg(long)]
        release: bool,
        #[arg(long, hide = true, value_parser = ["book", "chart"])]
        freeze_stock: Option<String>,
    },
    /// Synchronize platform projects and generated declarations.
    Sync {
        #[arg(long)]
        platform: Option<String>,
    },
    /// Type-check the app.
    Analyze {
        /// Select the plugin platform for type checking.
        #[arg(long)]
        platform: Option<String>,
        /// Path to a native TypeScript executable.
        #[arg(long)]
        tsc: Option<PathBuf>,
        /// Check the TypeScript fixture against the runtime's standard declarations.
        #[arg(long, conflicts_with = "platform")]
        standard: bool,
    },
    /// Add an Impasto package and optionally its corresponding Rust dependency.
    Add {
        package: String,
        #[arg(long)]
        rust: bool,
    },
    /// Run tests without opening a window.
    Test {
        /// Source file or trusted bytecode produced by this version of `impasto build`.
        bundle: Option<PathBuf>,
        /// Run the framework's TypeScript and TSX tests.
        #[arg(long, conflicts_with_all = ["bundle", "platform", "gpu"])]
        framework: bool,
        #[arg(long)]
        platform: Option<String>,
        #[arg(long)]
        trace: bool,
        /// Render and compare requested PNG snapshots.
        #[arg(long)]
        gpu: bool,
        /// Explicitly update the references used by screen.snapshot.
        #[arg(long, requires = "gpu")]
        update_snapshots: bool,
    },
    /// Assemble a relocatable SDK from the current Impasto executable.
    SdkDist { output: PathBuf },
    /// Resolve release script positions to the original source.
    Symbolicate {
        symbols: PathBuf,
        #[arg(required_unless_present_any = ["stack", "trace"], requires = "column")]
        line: Option<u32>,
        #[arg(requires = "line")]
        column: Option<u32>,
        /// Remap the QuickJS frames in a crash log.
        #[arg(long, conflicts_with_all = ["line", "column", "trace"])]
        stack: Option<PathBuf>,
        /// Annotate JavaScript stacks in a recorded Perfetto trace.
        #[arg(long, requires = "output", conflicts_with_all = ["line", "column"])]
        trace: Option<PathBuf>,
        /// Destination for the annotated Perfetto trace.
        #[arg(long, requires = "trace")]
        output: Option<PathBuf>,
    },
    /// Delete generated glue and build outputs.
    Clean,
    /// Open a standalone diagnostics client, optionally forwarding a USB device.
    Devtools(devtools::Options),
    /// Package-manager commands. Only `get` is implemented.
    Packages {
        #[command(subcommand)]
        command: PackagesCommand,
    },
}

#[derive(Subcommand)]
enum PackagesCommand {
    /// Resolve dependencies and synchronize the app.
    Get,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Create { name, with_rust } => finish(create::create(&name, with_rust)),
        Command::InitRust => finish(create::init_rust()),
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
            platform,
            installed_manifest,
            ios_simulator,
            export_options,
            debug,
            release: _,
            verbose,
        } => finish(build::build(
            &target,
            platform.as_deref(),
            build::BuildOptions {
                hot: None,
                debug,
                verbose,
                trace: false,
            },
            ios_simulator,
            export_options.as_deref(),
            installed_manifest.as_deref(),
        )),
        Command::Pack {
            entry,
            output,
            release,
            freeze_stock,
        } => finish(build::pack(
            &entry,
            &output,
            &build::BuildOptions {
                hot: None,
                debug: !release,
                verbose: false,
                trace: false,
            },
            freeze_stock.as_deref(),
        )),
        Command::Sync { platform } => finish(sync::run(platform.as_deref())),
        Command::Analyze {
            platform,
            tsc,
            standard,
        } => finish(if standard {
            analyze::standard(tsc.as_deref())
        } else {
            analyze::analyze(platform.as_deref(), tsc.as_deref())
        }),
        Command::Add { package, rust } => finish(packages::add(&package, rust)),
        Command::Test {
            bundle,
            framework,
            platform,
            trace,
            gpu,
            update_snapshots,
        } => finish(test::test(
            bundle,
            framework,
            platform.as_deref(),
            trace,
            gpu,
            update_snapshots,
        )),
        Command::SdkDist { output } => finish(distribute::write(&output)),
        Command::Symbolicate {
            symbols,
            line,
            column,
            stack,
            trace,
            output,
        } => finish(symbols::symbolicate(
            &symbols,
            line.zip(column),
            stack.as_deref(),
            trace.as_deref().zip(output.as_deref()),
        )),
        Command::Clean => finish(clean::clean()),
        Command::Devtools(options) => finish(devtools::run(options)),
        Command::Packages { command } => match command {
            PackagesCommand::Get => finish(packages::get()),
        },
    }
}

fn finish(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("impasto: {message:#}");
            ExitCode::from(1)
        }
    }
}

/// Framework checkout when this binary was built from one. Used so `impasto run`
/// inside the checkout still opens the gallery.
fn framework_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    if root.join(GALLERY_DIR).join("app.toml").is_file() {
        root.canonicalize().ok()
    } else {
        None
    }
}
