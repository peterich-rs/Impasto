//! macOS host. `--js <script>` loads source or trusted `.bin` output from `impasto build`.
//! `--url` is the Linking launch URL, written before the script evaluates.

#![allow(
    unsafe_code,
    reason = "AppKit and Metal are called through objc2 from this crate"
)]

mod a11y;
mod app;
#[allow(
    dead_code,
    reason = "image bytes are read when the host resolves an asset"
)]
mod assets;
mod native;
mod pixels;
mod pointer;
mod surface;
mod text_client;
mod vsync;

use std::ffi::c_char;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;

pub fn main() -> ExitCode {
    ExitCode::from(exit_status())
}

/// Launches the host on the AppKit main thread and reports its process status.
pub fn exit_status() -> u8 {
    exit_status_with_url_handler(None)
}

#[doc(hidden)]
pub type OpenUrl = extern "C" fn(*const c_char) -> bool;

/// Launches with a native URL interceptor invoked on the AppKit main thread.
pub fn exit_status_with_url_handler(open_url: Option<OpenUrl>) -> u8 {
    exit_status_with_callbacks(open_url, None)
}

#[doc(hidden)]
pub use native::Callbacks;

/// Launches the host with native plugin callbacks on the AppKit main thread.
#[doc(hidden)]
pub fn exit_status_with_callbacks(open_url: Option<OpenUrl>, callbacks: Option<Callbacks>) -> u8 {
    match launch(open_url, callbacks) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            app::exit_code(&error)
        }
    }
}

fn launch(open_url: Option<OpenUrl>, callbacks: Option<Callbacks>) -> Result<(), app::RunError> {
    let native = native::Host::new(open_url, callbacks);
    let loaded = load().map_err(app::RunError::Window)?;
    objc2::rc::autoreleasepool(|_| {
        app::run(
            loaded.launch,
            loaded.assets,
            &loaded.font,
            loaded.dump_list,
            loaded.url,
            native,
            &loaded.title,
        )
    })
}

struct Loaded {
    launch: app::Launch,
    assets: PathBuf,
    font: Vec<u8>,
    dump_list: bool,
    url: Option<String>,
    title: String,
}

fn load() -> anyhow::Result<Loaded> {
    let paths = Paths::from_args()?;
    let font = std::fs::read(&paths.font)
        .with_context(|| format!("failed to read font {}", paths.font.display()))?;
    let script = paths.script.as_ref().context("--js needs a script path")?;
    let source = std::fs::read(script)
        .with_context(|| format!("failed to read script {}", script.display()))?;
    let launch = app::Launch::Script {
        name: script.display().to_string(),
        source,
        modules: script
            .parent()
            .map(|dir| dir.join("app.modules"))
            .filter(|path| path.is_file())
            .map(std::fs::read_to_string)
            .transpose()?
            .map(|text| {
                text.split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            }),
        bytecode: script.extension().is_some_and(|ext| ext == "bin"),
    };
    Ok(Loaded {
        launch,
        assets: paths.assets,
        font,
        dump_list: paths.dump_list,
        url: paths.url,
        title: paths.title,
    })
}

struct Paths {
    assets: PathBuf,
    font: PathBuf,
    script: Option<PathBuf>,
    dump_list: bool,
    url: Option<String>,
    title: String,
}

impl Paths {
    fn from_args() -> anyhow::Result<Self> {
        let resources = std::env::current_exe()?
            .parent()
            .and_then(|dir| dir.parent())
            .map(|dir| dir.join("Resources"));
        let resources =
            resources.filter(|dir| dir.join("app.bin").is_file() || dir.join("app.js").is_file());
        let mut assets = resources.as_ref().map_or_else(
            || PathBuf::from("crates/embedder/macos/assets"),
            |dir| dir.join("assets"),
        );
        let mut font = PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf");
        let mut script = resources.map(|dir| {
            let bin = dir.join("app.bin");
            if bin.is_file() {
                bin
            } else {
                dir.join("app.js")
            }
        });
        let mut dump_list = false;
        let mut url = None;
        let mut title = bundle_title().unwrap_or_else(|| "Impasto".to_owned());
        let mut positional = 0u8;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--js" {
                let Some(path) = args.next() else {
                    anyhow::bail!("--js needs a script path");
                };
                script = Some(PathBuf::from(path));
                continue;
            }
            if arg == "--title" {
                title = args.next().context("--title needs a window title")?;
                continue;
            }
            if arg == "--dump-list" {
                dump_list = true;
                continue;
            }
            if arg == "--url" {
                let Some(value) = args.next() else {
                    anyhow::bail!("--url needs a URL");
                };
                url = app::nonempty_url(&value);
                continue;
            }
            match positional {
                0 => assets = PathBuf::from(arg),
                _ => font = PathBuf::from(arg),
            }
            positional = positional.saturating_add(1);
        }
        Ok(Self {
            assets,
            font,
            script,
            dump_list,
            url,
            title,
        })
    }
}

pub use embedder::install_plugins;

fn bundle_title() -> Option<String> {
    use objc2_foundation::{NSBundle, NSString};
    let bundle = NSBundle::mainBundle();
    ["CFBundleDisplayName", "CFBundleName"]
        .into_iter()
        .find_map(|key| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))?
                .downcast::<NSString>()
                .ok()
                .map(|value| value.to_string())
        })
}
