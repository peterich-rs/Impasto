//! macOS host. `--js <script>` loads that script in the QuickJS session.
//! `--url` or a `scheme://` argument is the Linking launch URL.

mod a11y;
mod app;
#[allow(dead_code)]
mod assets;
mod launch;
mod pixels;
mod pointer;
mod surface;
mod text_client;
mod vsync;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;

fn main() -> ExitCode {
    match launch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(app::exit_code(&error))
        }
    }
}

fn launch() -> Result<(), app::RunError> {
    let (launch, assets, font, dump_list) = load().map_err(app::RunError::Window)?;
    objc2::rc::autoreleasepool(|_| app::run(launch, assets, &font, dump_list))
}

fn load() -> anyhow::Result<(app::Launch, PathBuf, Vec<u8>, bool)> {
    let paths = Paths::from_args()?;
    let font = std::fs::read(&paths.font)
        .with_context(|| format!("failed to read font {}", paths.font.display()))?;
    let script = paths.script.as_ref().context("--js needs a script path")?;
    let source = std::fs::read_to_string(script)
        .with_context(|| format!("failed to read script {}", script.display()))?;
    let launch = app::Launch::Script {
        name: script.display().to_string(),
        source,
    };
    Ok((launch, paths.assets, font, paths.dump_list))
}

struct Paths {
    assets: PathBuf,
    font: PathBuf,
    script: Option<PathBuf>,
    dump_list: bool,
}

impl Paths {
    fn from_args() -> anyhow::Result<Self> {
        let mut assets = PathBuf::from("hosts/macos/assets");
        let mut font = PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf");
        let mut script = None;
        let mut dump_list = false;
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
            if arg == "--dump-list" {
                dump_list = true;
                continue;
            }
            if arg == "--url" {
                if args.next().is_none() {
                    anyhow::bail!("--url needs a URL");
                }
                continue;
            }
            if launch::looks_like_url(&arg) {
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
        })
    }
}
