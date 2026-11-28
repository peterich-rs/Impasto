use crate::{
    android_watch,
    app::App,
    build::{self, BuildOptions},
    dev::{self, Delivery, Event, Running, Target},
    toolchain,
};
use anyhow::{Context, Result};
use std::{
    fs,
    io::IsTerminal,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
};

pub struct RunOptions {
    pub device: Option<String>,
    pub release: bool,
    pub verbose: bool,
    pub trace: bool,
    pub no_watch: bool,
    pub devtools: bool,
}

pub fn run(options: RunOptions) -> Result<()> {
    anyhow::ensure!(
        !options.devtools || std::env::var("IMPASTO_DEVTOOLS").as_deref() != Ok("0"),
        "--devtools requires the DevTools agent to be enabled"
    );
    let app = App::current()?;
    let device = options
        .device
        .as_deref()
        .unwrap_or(if cfg!(target_os = "macos") {
            "macos"
        } else {
            "android"
        });
    let watch = !options.release && !options.no_watch;
    let settings = BuildOptions {
        debug: !options.release,
        hot: watch.then(Default::default),
        verbose: options.verbose,
        trace: options.trace,
    };
    if device == "ios" || device.starts_with("ios:") {
        return crate::ios::run(
            app,
            &settings,
            device.strip_prefix("ios:"),
            watch,
            options.devtools,
        );
    }
    if device != "macos" {
        let serial = if device == "android" {
            crate::devtools::android_serial()?
        } else {
            device.to_owned()
        };
        let target = Android { serial };
        if watch {
            return dev::run(&target, app, &settings, options.devtools);
        }
        return target
            .deploy(&app, &settings, options.devtools, None)
            .map(|_| ());
    }
    if watch {
        #[cfg(unix)]
        return dev::run(&Macos, app, &settings, options.devtools);
        #[cfg(not(unix))]
        anyhow::bail!("macOS 开发会话需要 Unix 系统");
    }
    let application = build::macos(&app, &settings)?;
    let mut command = macos_command(&app, &application.path, options.trace)?;
    command.stdin(Stdio::null());
    if !std::io::stdout().is_terminal() {
        command.stdout(Stdio::null());
    }
    if !std::io::stderr().is_terminal() {
        command.stderr(Stdio::null());
    }
    let child = command.spawn()?;
    if options.devtools {
        crate::devtools::launch_native(Some(&child.id().to_string()))?;
    }
    Ok(())
}

#[cfg(unix)]
struct Macos;

#[cfg(unix)]
impl Target for Macos {
    fn platform(&self) -> &'static str {
        "macos"
    }

    fn launch(
        &self,
        app: &App,
        options: &BuildOptions,
        events: &mpsc::Sender<Event>,
        attach: bool,
    ) -> Result<Running> {
        use std::os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        };

        let artifact = build::macos(app, options)?;
        let directory = tempfile::Builder::new().prefix("ip-").tempdir_in("/tmp")?;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
        let path = directory.path().join("dev.sock");
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let wake = path.clone();
        let token = dev::token()?;
        let app_id = app.name.clone();
        let channel = dev::Channel::start(
            move || {
                listener
                    .accept()
                    .map(|(stream, _)| dev::Stream::Unix(stream))
                    .map_err(Into::into)
            },
            Some(Box::new(move || {
                let _ = UnixStream::connect(wake);
                drop(directory);
            })),
            token.clone(),
            app_id.clone(),
            events.clone(),
        )?;
        let child = macos_command(app, &artifact.path, options.trace)?
            .env("IMPASTO_DEV_SOCKET", path)
            .env("IMPASTO_DEV_SESSION", token)
            .env("IMPASTO_DEV_APP_ID", app_id)
            .env(
                "IMPASTO_DEV_VERBOSE",
                if options.verbose { "1" } else { "0" },
            )
            .spawn()?;
        let pid = child.id();
        let child = dev::Child::wait(child, events.clone())?;
        if attach {
            crate::devtools::launch_native(Some(&pid.to_string()))?;
        }
        Ok(Running {
            manifest: artifact.plugin_manifest,
            source: fs::read_to_string(artifact.path.join("Contents/Resources/app.js")).ok(),
            channel: Delivery::Socket(channel),
            child: Some(child),
        })
    }
}

fn macos_command(app: &App, application: &Path, trace: bool) -> Result<Command> {
    let mut command = Command::new(application.join("Contents/MacOS/impasto"));
    command.current_dir(&app.root);
    if trace {
        command.env("IMPASTO_TRACE", "1");
    }
    command.env("IMPASTO_DEVTOOLS_APP_NAME", &app.name);
    command.env("IMPASTO_DEVTOOLS_PROJECT", &app.root);
    command.env("IMPASTO_DEVTOOLS_FRAMEWORK", toolchain::framework()?);
    command.env(
        "IMPASTO_DEVTOOLS_SOURCE_MAP",
        application.join("Contents/Resources/app.js.map"),
    );
    Ok(command)
}

struct Android {
    serial: String,
}

impl Android {
    fn command(&self) -> Command {
        android_watch::adb(&self.serial)
    }

    fn deploy(
        &self,
        app: &App,
        options: &BuildOptions,
        attach: bool,
        token: Option<&str>,
    ) -> Result<(build::PlatformArtifact, String)> {
        let artifact = build::apk(app, options)?;
        let package = app.android_id();
        let mut install = self.command();
        install.args(["install", "-r"]).arg(&artifact.path);
        toolchain::run(install, "安装 APK")?;
        let resolved = self
            .command()
            .args([
                "shell",
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
                "-p",
                &package,
            ])
            .output()?;
        let output = String::from_utf8(resolved.stdout)?;
        let activity = output
            .lines()
            .last()
            .map(str::trim)
            .filter(|activity| {
                resolved.status.success()
                    && activity
                        .split_once('/')
                        .is_some_and(|(name, class)| name == package && !class.is_empty())
            })
            .with_context(|| format!("找不到 {package} 的 Android 启动入口"))?;
        if options.debug {
            android_watch::configure_session(&self.serial, &package, token, options.verbose)?;
        }
        let config = if options.debug {
            crate::devtools::device_config()?
        } else {
            None
        };
        if let Some((port, token)) = &config {
            crate::devtools::configure_android(&|| self.command(), &package, *port, token)?;
        }
        let mut start = self.command();
        start.args(["shell", "am", "start", "-S", "-n", activity]);
        toolchain::run(start, "启动 Android 应用")?;
        if let Some(config) = config {
            let id = crate::devtools::register_device(
                devtools_protocol::DeviceKind::Android,
                &self.serial,
                &package,
                config,
                &app.root,
                &app.root.join("build/android/bundle/debug"),
            )?;
            if attach {
                crate::devtools::launch_native(Some(&id))?;
            }
        }
        Ok((artifact, activity.to_owned()))
    }
}

impl Target for Android {
    fn platform(&self) -> &'static str {
        "android"
    }

    fn launch(
        &self,
        app: &App,
        options: &BuildOptions,
        events: &mpsc::Sender<Event>,
        attach: bool,
    ) -> Result<Running> {
        let token = dev::token()?;
        let (artifact, activity) = self.deploy(app, options, attach, Some(&token))?;
        let serial = self.serial.clone();
        let package = app.android_id();
        let channel = if android_watch::uses_file_transport() {
            eprintln!("impasto: 自定义 ADB 服务器使用文件调试通道");
            Delivery::Files(Box::new(android_watch::Files::new(serial, package, token)))
        } else {
            let app_id = package.clone();
            let mut attempts = 0;
            Delivery::Socket(dev::Channel::start(
                move || {
                    let result = android_watch::open(&serial, &package);
                    if result.is_ok() {
                        attempts = 0;
                    } else {
                        attempts += 1;
                        if attempts % 4 == 0 {
                            let running = android_watch::adb(&serial)
                                .args(["shell", "pidof", &package])
                                .output()
                                .is_ok_and(|output| {
                                    output.status.success() && !output.stdout.is_empty()
                                });
                            if !running {
                                let _ = android_watch::adb(&serial)
                                    .args(["shell", "am", "start", "-n", &activity])
                                    .stdout(Stdio::null())
                                    .stderr(Stdio::null())
                                    .status();
                            }
                        }
                    }
                    result
                },
                None,
                token,
                app_id,
                events.clone(),
            )?)
        };
        Ok(Running {
            manifest: artifact.plugin_manifest,
            source: fs::read_to_string(app.root.join("build/android/bundle/debug/app.js")).ok(),
            channel,
            child: None,
        })
    }
}
