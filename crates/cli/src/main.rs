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
mod size;
mod symbols;
mod sync;
mod test;
mod toolchain;
mod typescript;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[cfg(test)]
const GALLERY_DIR: &str = "examples/gallery-solid";

#[derive(Parser)]
#[command(name = "impasto", about = "Build and run an Impasto app")]
struct Cli {
    /// Project directory containing app.toml.
    #[arg(long, global = true)]
    project: Option<PathBuf>,
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
        /// Generate a native plugin instead of an application.
        #[arg(long, conflicts_with = "with_rust")]
        plugin: bool,
        /// Platform projects to generate for a plugin.
        #[arg(long, value_delimiter = ',', requires = "plugin")]
        platforms: Vec<String>,
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
        /// Write bytecode chunk sizes and rendered module contributions as JSON.
        #[arg(long)]
        analyze_size: bool,
        /// Save the bundle size report to this JSON path.
        #[arg(long)]
        size_json: Option<PathBuf>,
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
        /// Inline a JavaScript expression for a global identifier.
        #[arg(long = "define", value_name = "KEY=JS_EXPR")]
        defines: Vec<String>,
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
        #[arg(long, conflicts_with_all = ["platform", "all_targets", "deny_warnings"])]
        standard: bool,
        /// Check each target declared by the project.
        #[arg(long, conflicts_with = "platform")]
        all_targets: bool,
        /// Treat compiler warnings as errors.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Add an Impasto package and optionally its corresponding Rust dependency.
    Add {
        package: String,
        #[arg(long)]
        rust: bool,
    },
    /// Remove an explicitly declared package and refresh generated glue.
    Remove { package: String },
    /// Refresh the locked revisions of Git dependencies.
    Update {
        package: Option<String>,
        #[arg(long, requires = "package")]
        tag: Option<String>,
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
        /// Re-run affected test entries when a source file changes.
        #[arg(long)]
        watch: bool,
        /// Run test cases whose full names contain this text.
        #[arg(short = 'k', long)]
        filter: Option<String>,
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
    /// Package-manager commands.
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
    let cli = Cli::parse();
    if let Some(project) = cli.project
        && let Err(error) = std::env::set_current_dir(&project)
    {
        return finish(Err(anyhow::anyhow!(
            "无法进入 {}: {error}",
            project.display()
        )));
    }
    match cli.command {
        Command::Create {
            name,
            with_rust,
            plugin,
            platforms,
        } => finish(if plugin {
            create::plugin(&name, &platforms)
        } else {
            create::create(&name, with_rust)
        }),
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
            analyze_size,
            size_json,
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
            size::Request {
                summary: analyze_size,
                path: size_json.as_deref(),
            },
        )),
        Command::Pack {
            entry,
            output,
            release,
            defines,
        } => finish(build::pack(
            &entry,
            &output,
            &build::BuildOptions {
                hot: None,
                debug: !release,
                verbose: false,
                trace: false,
            },
            &defines,
        )),
        Command::Sync { platform } => finish(sync::run(platform.as_deref())),
        Command::Analyze {
            platform,
            tsc,
            standard,
            all_targets,
            deny_warnings,
        } => finish(if standard {
            analyze::standard(tsc.as_deref())
        } else {
            analyze::analyze(
                platform.as_deref(),
                tsc.as_deref(),
                all_targets,
                deny_warnings,
            )
        }),
        Command::Add { package, rust } => finish(packages::add(&package, rust)),
        Command::Remove { package } => finish(packages::remove(&package)),
        Command::Update { package, tag } => {
            finish(packages::update(package.as_deref(), tag.as_deref()))
        }
        Command::Test {
            bundle,
            framework,
            platform,
            trace,
            gpu,
            update_snapshots,
            watch,
            filter,
        } => finish(test::test(
            bundle,
            framework,
            platform.as_deref(),
            test::Options {
                trace,
                gpu,
                update: update_snapshots,
                watch,
                filter,
            },
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

/// Framework checkout used to build this executable.
fn framework_root() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    if root.join("Cargo.lock").is_file() && root.join("crates/engine/Cargo.toml").is_file() {
        root.canonicalize().ok()
    } else {
        None
    }
}
