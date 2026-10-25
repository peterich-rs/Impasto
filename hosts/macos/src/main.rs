//! macOS host. Version 1 scene bytes become a draw list before the window is shown.
//! `--js <script>` loads that script in the QuickJS session instead.

mod a11y;
mod app;
#[allow(dead_code)]
mod assets;
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
    let (launch, assets, font) = load().map_err(app::RunError::Window)?;
    objc2::rc::autoreleasepool(|_| app::run(launch, assets, &font))
}

fn load() -> anyhow::Result<(app::Launch, PathBuf, Vec<u8>)> {
    let paths = Paths::from_args()?;
    let font = std::fs::read(&paths.font)
        .with_context(|| format!("failed to read font {}", paths.font.display()))?;
    let launch = if let Some(script) = &paths.script {
        let source = std::fs::read_to_string(script)
            .with_context(|| format!("failed to read script {}", script.display()))?;
        app::Launch::Script {
            name: script.display().to_string(),
            source,
        }
    } else {
        let scene_bytes = std::fs::read(&paths.scene)
            .with_context(|| format!("failed to read scene {}", paths.scene.display()))?;
        let scene = protocol::decode(&scene_bytes).context("scene bytes are not version 1")?;
        app::Launch::Fixture(protocol::scene_to_draw_list(&scene))
    };
    Ok((launch, paths.assets, font))
}

struct Paths {
    scene: PathBuf,
    assets: PathBuf,
    font: PathBuf,
    script: Option<PathBuf>,
}

impl Paths {
    fn from_args() -> anyhow::Result<Self> {
        let mut scene = PathBuf::from("protocol/tests/fixtures/counter.bin");
        let mut assets = PathBuf::from("hosts/macos/assets");
        let mut font = PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf");
        let mut script = None;
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
            if script.is_some() {
                match positional {
                    0 => assets = PathBuf::from(arg),
                    _ => font = PathBuf::from(arg),
                }
            } else {
                match positional {
                    0 => scene = PathBuf::from(arg),
                    1 => assets = PathBuf::from(arg),
                    _ => font = PathBuf::from(arg),
                }
            }
            positional = positional.saturating_add(1);
        }
        Ok(Self {
            scene,
            assets,
            font,
            script,
        })
    }
}
