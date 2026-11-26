//! Installed resources, JavaScript entry point and enabled host modules.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use embedder::{BootError, ImageDecoder, Launch, SessionError};
use engine::session::Module;
use objc2_foundation::{NSBundle, NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};

#[path = "decode.rs"]
mod decode;
const FONT_FAMILY: &str = "Sans";
// This is the same Apache-2.0 licensed Roboto font shipped in Android.
const FONT: &[u8] = include_bytes!("../../android/assets/fonts/Sans.ttf");

pub(crate) struct Bundle {
    pub launch: Launch,
    pub font_family: String,
    pub font: Vec<u8>,
    pub decoder: Arc<dyn ImageDecoder>,
    pub modules: Vec<Module>,
    pub devtools: bool,
    pub plugin_services: embedder::PluginServices,
}

#[allow(
    unsafe_code,
    reason = "QuickJS loads trusted compiler output from the installed bundle"
)]
pub(crate) fn prepare(
    root: PathBuf,
    native_channel: impasto::os::NativeChannel,
) -> Result<Bundle, BootError> {
    objc2::rc::autoreleasepool(|_| {
        let enabled = enabled_modules(&root)?;
        let devtools = is_enabled(&enabled, "devtools");
        #[cfg(feature = "bundled-plugins")]
        {
            static INSTALLED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
            INSTALLED
                .get_or_init(|| {
                    embedder::install_bundled_plugins(impasto_bundled_plugins::register)
                        .map_err(|error| error.to_string())
                })
                .as_ref()
                .map_err(|error| BootError::Platform(error.clone()))?;
        }
        let program = load_program(&root)?;

        let launch = Launch::Session(Box::new(move |open, host| {
            let host = host.restrict(&enabled);
            match program {
                Program::Source(bytes) => {
                    engine::js::open_program(&bytes, "app.js", open.shaper, open.log, host)
                }
                Program::Bytecode(bytes) => {
                    // SAFETY: load_program accepts bytecode only from the installed
                    // app bundle, where the packaging tool stores its compiler output.
                    unsafe {
                        engine::js::open_trusted_bytecode(
                            &bytes,
                            "app.js",
                            open.shaper,
                            open.log,
                            host,
                        )
                    }
                }
            }
            .map_err(|error| match error {
                engine::js::Error::Runtime => SessionError::Runtime,
                engine::js::Error::Script(text) => SessionError::Script(text),
            })
        }));

        Ok(Bundle {
            launch,
            font_family: FONT_FAMILY.into(),
            font: FONT.to_vec(),
            decoder: Arc::new(decode::Decoder::new(root.join("assets"))),
            modules: Vec::new(),
            devtools,
            plugin_services: embedder::PluginServices {
                initial_lifecycle: Default::default(),
                native_channel: Some(native_channel),
                paths: embedder::AppPaths::new(
                    sandbox_directory(NSSearchPathDirectory::DocumentDirectory)
                        .ok()
                        .map(PathBuf::from),
                    sandbox_directory(NSSearchPathDirectory::CachesDirectory)
                        .ok()
                        .map(PathBuf::from),
                    Some(std::env::temp_dir()),
                )
                .with_data_dir(
                    sandbox_directory(NSSearchPathDirectory::ApplicationSupportDirectory)
                        .ok()
                        .map(PathBuf::from),
                ),
            },
        })
    })
}

enum Program {
    Source(Vec<u8>),
    Bytecode(Vec<u8>),
}

fn load_program(root: &Path) -> Result<Program, BootError> {
    let source = root.join("app.js");
    let bytecode = root.join("app.bin");
    match fs::metadata(&bytecode) {
        Ok(_) => {
            match fs::symlink_metadata(&source) {
                Ok(_) => {
                    return Err(BootError::Platform(
                        "应用包同时包含 app.js 和 app.bin".into(),
                    ));
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(read_error(&source, error)),
            }
            let path = bytecode
                .canonicalize()
                .map_err(|error| read_error(&bytecode, error))?;
            let bundle = PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
            let bundle = bundle
                .canonicalize()
                .map_err(|error| read_error(&bundle, error))?;
            if !path.starts_with(bundle) {
                return Err(BootError::Platform(
                    "app.bin 必须位于已安装的应用包中".into(),
                ));
            }
            fs::read(&path)
                .map(Program::Bytecode)
                .map_err(|error| read_error(&path, error))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => fs::read(&source)
            .map(Program::Source)
            .map_err(|error| read_error(&source, error)),
        Err(error) => Err(read_error(&bytecode, error)),
    }
}

fn read_error(path: &Path, error: std::io::Error) -> BootError {
    BootError::Platform(format!("读取 {} 失败。{error}", path.display()))
}

fn enabled_modules(root: &Path) -> Result<Vec<String>, BootError> {
    let path = root.join("app.modules");
    let manifest = match fs::read_to_string(&path) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(read_error(&path, error)),
    };
    Ok(manifest
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

fn is_enabled(enabled: &[String], name: &str) -> bool {
    enabled.iter().any(|item| item == name)
}

fn sandbox_directory(directory: NSSearchPathDirectory) -> Result<String, BootError> {
    let url = NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            directory,
            NSSearchPathDomainMask::UserDomainMask,
            None,
            true,
        )
        .map_err(|error| BootError::Platform(format!("准备应用存储目录失败。{error}")))?;
    url.path()
        .map(|path| path.to_string())
        .ok_or_else(|| BootError::Platform("应用存储目录没有本地路径".into()))
}
