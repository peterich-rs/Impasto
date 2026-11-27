//! Drives application tests in isolated QuickJS UI sessions.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use engine::{
    input::{Button, Buttons, Phase, PointerEvent, PointerKind},
    js::JsSession,
    semantics::{SemanticsRole, SemanticsUpdate},
    session::{
        AssetReader, BackGesture, Delivery, FrameTime, FrameTrace, HostFuture, InputEvent, IoEvent,
        Method, MethodExec, Metrics, Module, ModuleConfig, ModuleExports, ModuleRegistry,
        PluginResult, PluginValue, ScriptExecution, Session, Settle, Work,
    },
    text::MonoShaper,
};

#[cfg(feature = "headless-gpu")]
mod snapshots;

/// Where a test may compare rendered output with checked-in PNGs.
#[derive(Debug, Clone)]
pub struct SnapshotOptions {
    /// Directory specific to this entry.
    pub root: PathBuf,
    /// Save actual pixels as the new reference.
    pub update: bool,
}

#[cfg(feature = "linker")]
const TEST_MODULE: &str = "impasto:test";
const DRIVER_MODULE: &str = "impasto:test/driver";

/// Configure in-memory test imports for the linker.
#[cfg(feature = "linker")]
pub fn install(options: &mut bundler::BundleOptions) {
    options
        .virtual_modules
        .insert(TEST_MODULE.to_owned(), include_str!("test.js").to_owned());
    options.externals.insert(DRIVER_MODULE.to_owned());
}

/// Exposes text fixtures relative to one test entry inside the project root.
///
/// # Errors
///
/// Reports an invalid project root or an entry without a parent directory.
pub fn fixture_module(root: &Path, entry: &Path) -> Result<Module> {
    let root = root.canonicalize()?;
    let base = entry
        .parent()
        .context("test entry has no directory")?
        .to_owned();
    Ok(Module {
        name: "ImpastoTestFixtures",
        specifier: "impasto:test/fixtures",
        methods: vec![Method::sync("readText", move |args| {
            let Some(PluginValue::Str(name)) = args.first() else {
                return Err("readText requires a relative fixture path".to_owned());
            };
            let path = base
                .join(name)
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if Path::new(name).is_absolute() || !path.starts_with(&root) {
                return Err("test fixture must be inside the project".to_owned());
            }
            fs::read_to_string(path)
                .map(PluginValue::Str)
                .map_err(|error| error.to_string())
        })],
        exports: ModuleExports::default(),
    })
}

/// Test cases and accumulated work reported by an isolated session.
#[derive(Debug, Default)]
pub struct Report {
    /// Number of registered cases that ran to completion.
    pub passed: usize,
    /// Work performed in the frames driven by the test.
    pub frames: FrameTrace,
}

/// Formats the result consistently for the CLI and application-linked test hosts.
pub fn format_report(path: &Path, report: &Report, trace: bool) -> String {
    let mut text = format!("{} ({} tests)", path.display(), report.passed);
    if trace {
        use std::fmt::Write as _;
        let frame = &report.frames;
        let _ = write!(
            text,
            "\n  created_nodes={} ffi_calls={} ffi_batches={} style_resolves={} repaint_boundaries_dirtied={} updates={} sets={} layout_passes={} text_shapings={} pointers={} edits={}",
            frame.nodes_created,
            frame.ffi_calls,
            frame.ffi_batches,
            frame.style_resolves,
            frame.repaint_boundaries_dirtied,
            frame.updates,
            frame.sets,
            frame.layout_passes,
            frame.text_shaped,
            frame.pointers,
            frame.edits,
        );
    }
    text
}

/// Run a linked test or application smoke test with optional native modules.
///
/// # Errors
///
/// Returns a script fault, assertion failure, or an error if a test stalls.
pub fn run(
    code: &str,
    name: &str,
    modules: Vec<Module>,
    snapshots: Option<SnapshotOptions>,
) -> Result<Report> {
    run_with_assets(code, name, modules, snapshots, None)
}

/// Run an application test with its packaged asset directory.
///
/// # Errors
/// Returns the same script or driver failures as [run].
pub fn run_with_assets(
    code: &str,
    name: &str,
    modules: Vec<Module>,
    snapshots: Option<SnapshotOptions>,
    assets: Option<PathBuf>,
) -> Result<Report> {
    run_filtered(code, name, modules, snapshots, assets, None)
}

/// Run a test entry and select case names containing the supplied substring.
///
/// # Errors
/// Returns the same script or driver failures as [run].
pub fn run_filtered(
    code: &str,
    name: &str,
    mut modules: Vec<Module>,
    snapshots: Option<SnapshotOptions>,
    assets: Option<PathBuf>,
    filter: Option<String>,
) -> Result<Report> {
    #[cfg(not(feature = "headless-gpu"))]
    if snapshots.is_some() {
        bail!("GPU snapshots require the headless-gpu feature");
    }
    let (actions, pending) = mpsc::channel();
    let io = actions.clone();
    let routed = actions.clone();
    let (_runtime, methods) = TestExecutor::new()?;
    let report = actions.clone();
    modules.push(Module {
        name: "ImpastoTestDriver",
        specifier: DRIVER_MODULE,
        methods: vec![
            Method::sync("filter", move |_| {
                Ok(filter.clone().map_or(PluginValue::Null, PluginValue::Str))
            }),
            Method::sync("report", move |args| {
                let event = Command::Report {
                    kind: text(args, 0)?.to_owned(),
                    value: text(args, 1).unwrap_or_default().to_owned(),
                };
                report.send(event).map_err(|error| error.to_string())?;
                Ok(PluginValue::Null)
            }),
            Method::promise("command", move |args, settle, _| {
                let _ = actions.send(Command::Request {
                    args: args.to_vec(),
                    settle,
                });
            }),
        ],
        exports: ModuleExports::default(),
    });
    let asset_reader: Option<AssetReader> = assets.map(|assets| {
        Arc::new(move |name: &str, limit: usize| {
            let root = assets.canonicalize().map_err(|error| error.to_string())?;
            let path = root
                .join(name)
                .canonicalize()
                .map_err(|error| error.to_string())?;
            if !path.starts_with(&root) || !path.is_file() {
                return Err("asset is outside the package".to_owned());
            }
            let file = fs::File::open(path).map_err(|error| error.to_string())?;
            let length = file.metadata().map_err(|error| error.to_string())?.len();
            let length = usize::try_from(length).map_err(|error| error.to_string())?;
            engine::session::read_asset_bytes(file, length, limit)
        }) as AssetReader
    });
    let transport_started = Instant::now();
    let config = ModuleConfig {
        asset_reader,
        registry: ModuleRegistry::new(modules).into(),
        script_execution: ScriptExecution::HeadlessTest,
        // Test frames can jump ahead to JavaScript timers while native work runs on real threads.
        transport_clock: Some(Arc::new(move || {
            i64::try_from(transport_started.elapsed().as_millis()).unwrap_or(i64::MAX)
        })),
        exec: Arc::new(methods),
        post_delivery: Some(Arc::new(move |delivery| {
            let _ = routed.send(Command::Delivery(delivery));
        })),
        settle: Arc::new(move |token| {
            let io = io.clone();
            Settle::new(move |result| {
                let _ = io.send(Command::Event(IoEvent::Plugin { token, result }));
            })
        }),
        ..ModuleConfig::disconnected()
    };
    let log = Box::new(|level, line: &str| {
        if level >= 3 {
            eprintln!("{line}");
        }
    });
    #[cfg(feature = "headless-gpu")]
    let snapshots = snapshots.map(snapshots::Snapshots::new);
    #[cfg(feature = "headless-gpu")]
    let mut session = if let Some(snapshots) = &snapshots {
        JsSession::open_with(
            code,
            name,
            raster::ParleyShaper::new(snapshots.fonts.clone()),
            log,
            config,
        )?
    } else {
        JsSession::open_with(code, name, MonoShaper::default(), log, config)?
    };
    #[cfg(not(feature = "headless-gpu"))]
    let mut session = JsSession::open_with(code, name, MonoShaper::default(), log, config)?;
    session.metrics(&Metrics::new(360., 640., 2.));
    session.set_counters(true);
    let mut driver = Driver {
        session,
        now: 0.,
        presented: 0.,
        virtual_clock: false,
        again: false,
        frames: FrameTrace::default(),
        #[cfg(feature = "headless-gpu")]
        snapshots,
    };
    let start = Instant::now();
    let mut registered = false;
    let mut report = Report::default();
    let mut current = String::new();
    let mut trace = FrameTrace::default();
    let limit = Duration::from_secs(10);
    driver.frame(0.)?;
    let mut smoke_frames = 0;
    loop {
        let remaining = limit.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            bail!("{current}: test timed out");
        }
        let command = match pending.try_recv() {
            Ok(command) => command,
            Err(_) if !registered && smoke_frames < 4 => {
                smoke_frames += 1;
                driver.frame(16.)?;
                continue;
            }
            Err(_) if !registered => break,
            Err(_) => {
                if !driver.virtual_clock && driver.again {
                    driver.frame(16.).with_context(|| current.clone())?;
                    continue;
                }
                let next = (!driver.virtual_clock)
                    .then(|| driver.session.next_wake_ms())
                    .flatten();
                if let Some(deadline) = next {
                    driver
                        .frame((deadline - driver.now).max(0.))
                        .with_context(|| current.clone())?;
                    continue;
                }
                match pending.recv_timeout(remaining) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => bail!("{current}: test timed out"),
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        bail!("{current}: test driver disconnected")
                    }
                }
            }
        };
        match command {
            Command::Report { kind, value } => match kind.as_str() {
                "start" => registered = true,
                "case" => current = value,
                "pass" => report.passed += 1,
                "done" => break,
                "fail" => bail!("{current}: {value}"),
                _ => bail!("invalid test event {kind}"),
            },
            Command::Request { args, settle } => {
                let result = match text(&args, 0) {
                    Ok("traceReset") => {
                        trace = driver.frames;
                        Ok(PluginValue::Null)
                    }
                    Ok("traceRead") => Ok(trace_values(&driver.frames, &trace)),
                    _ => driver.command(&args),
                };
                settle.send(match result {
                    Ok(value) => PluginResult::Done(value),
                    Err(message) => PluginResult::Failed { message },
                });
                if !driver.virtual_clock {
                    driver.frame(16.).with_context(|| current.clone())?;
                }
            }
            Command::Event(event) => {
                driver.session.io_event(event);
                driver.flush_delivery().with_context(|| current.clone())?;
            }
            Command::Delivery(delivery) => {
                driver.session.io_delivery(delivery);
                driver.flush_delivery().with_context(|| current.clone())?;
            }
        }
    }
    report.frames = driver.frames;
    Ok(report)
}

struct TestExecutor(Option<tokio::runtime::Runtime>);

impl TestExecutor {
    fn new() -> Result<(Self, AsyncMethods)> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let methods = AsyncMethods(runtime.handle().clone());
        Ok((Self(Some(runtime)), methods))
    }
}

impl Drop for TestExecutor {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

struct AsyncMethods(tokio::runtime::Handle);

impl MethodExec for AsyncMethods {
    fn spawn(&self, task: HostFuture) {
        drop(self.0.spawn(task));
    }

    fn worker(&self, settle: Settle, work: Work) {
        drop(self.0.spawn_blocking(move || settle.send(work())));
    }

    fn main_thread(&self, settle: Settle, work: Work) {
        settle.send(work());
    }

    fn now(&self, settle: Settle, result: PluginResult) {
        settle.send(result);
    }
}

enum Command {
    Event(IoEvent),
    Delivery(Delivery),
    Report {
        kind: String,
        value: String,
    },
    Request {
        args: Vec<PluginValue>,
        settle: Settle,
    },
}

struct Driver {
    session: JsSession,
    now: f64,
    presented: f64,
    virtual_clock: bool,
    again: bool,
    frames: FrameTrace,
    #[cfg(feature = "headless-gpu")]
    snapshots: Option<snapshots::Snapshots>,
}

fn text(args: &[PluginValue], index: usize) -> Result<&str, String> {
    match args.get(index) {
        Some(PluginValue::Str(value)) => Ok(value),
        _ => Err(format!("argument {} must be a string", index + 1)),
    }
}

fn number(args: &[PluginValue], index: usize) -> Result<f64, String> {
    match args.get(index) {
        Some(PluginValue::F64(value)) if value.is_finite() => Ok(*value),
        _ => Err(format!("argument {} must be a finite number", index + 1)),
    }
}

impl Driver {
    fn flush_delivery(&mut self) -> Result<()> {
        self.frame(if self.virtual_clock { 0. } else { 16. })
    }

    fn frame(&mut self, interval: f64) -> Result<()> {
        let time = if self.virtual_clock {
            self.now += interval;
            FrameTime {
                interval_ms: interval,
                ..FrameTime::at(self.now)
            }
        } else {
            let time = FrameTime::at(self.now);
            self.now += interval;
            time
        };
        let frame = self.session.frame(time);
        self.presented = time.target_ms;
        self.again = frame.again;
        #[cfg(feature = "headless-gpu")]
        if let Some(snapshots) = &mut self.snapshots
            && let Some(layers) = frame.layers
        {
            snapshots.layers = Some(layers);
        }
        let next = frame.phases.unwrap_or_default().trace;
        self.frames.nodes_created += next.nodes_created;
        self.frames.ffi_batches += next.ffi_batches;
        self.frames.ffi_calls += next.ffi_calls;
        self.frames.style_resolves += next.style_resolves;
        self.frames.repaint_boundaries_dirtied += next.repaint_boundaries_dirtied;
        self.frames.updates += next.updates;
        self.frames.sets += next.sets;
        self.frames.layout_passes += next.layout_passes;
        self.frames.text_shaped += next.text_shaped;
        self.frames.pointers += next.pointers;
        self.frames.edits += next.edits;
        if let Some(error) = self.session.take_script_error()
            && error != engine::Error::NoRoot.to_string()
        {
            bail!("{error}");
        }
        if let Some(fault) = self
            .session
            .take_faults()
            .into_iter()
            .find(|fault| fault.message != engine::Error::NoRoot.to_string())
        {
            bail!("{}: {}\n{}", fault.source, fault.message, fault.stack);
        }
        Ok(())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "pointer and IME arguments use the engine's platform types"
    )]
    fn command(&mut self, args: &[PluginValue]) -> Result<PluginValue, String> {
        match text(args, 0)? {
            "clockInstall" => {
                self.now = self.presented;
                self.virtual_clock = true;
            }
            "clockReset" => self.virtual_clock = false,
            "clockAdvance" => {
                let delta = number(args, 1)?;
                let target = self.now + delta;
                if !self.virtual_clock || !delta.is_finite() || delta < 0. || !target.is_finite() {
                    return Err("clock.advance requires an installed clock and a finite non-negative duration".to_owned());
                }
                let limit = Instant::now() + Duration::from_secs(10);
                while let Some(deadline) = self
                    .session
                    .next_wake_ms()
                    .filter(|deadline| *deadline <= target)
                {
                    if Instant::now() >= limit {
                        return Err("virtual clock advance timed out".to_owned());
                    }
                    self.frame((deadline - self.now).max(0.))
                        .map_err(|error| error.to_string())?;
                }
                self.frame(target - self.now)
                    .map_err(|error| error.to_string())?;
            }
            "frames" => {
                let count = number(args, 1)?;
                let step = number(args, 2)?;
                if !(1. ..=600.).contains(&count)
                    || count.fract() != 0.
                    || !(0. ..=1_000.).contains(&step)
                {
                    return Err("invalid frame count or interval".to_owned());
                }
                for _ in 0..count as u16 {
                    self.frame(step).map_err(|error| error.to_string())?;
                }
            }
            "viewport" => {
                let width = number(args, 1)?;
                let height = number(args, 2)?;
                let scale = number(args, 3)?;
                if !(1. ..=4096.).contains(&width)
                    || !(1. ..=4096.).contains(&height)
                    || !(0.5..=4.).contains(&scale)
                {
                    return Err("invalid test viewport".to_owned());
                }
                self.session
                    .metrics(&Metrics::new(width as f32, height as f32, scale as f32));
            }
            kind @ ("query" | "nodes") => {
                let role = text(args, 1)?;
                let label = text(args, 2).ok();
                let semantics = self.snapshot()?;
                let mut matched = semantics.nodes.iter().filter(|node| {
                    (role.is_empty() || role == role_name(node.role))
                        && label.is_none_or(|label| node.label == label)
                });
                if kind == "nodes" {
                    return Ok(PluginValue::List(
                        matched
                            .map(|node| {
                                PluginValue::Map(vec![
                                    ("id".to_owned(), PluginValue::F64(node.id.0 as f64)),
                                    ("label".to_owned(), PluginValue::Str(node.label.clone())),
                                ])
                            })
                            .collect(),
                    ));
                }
                let node = matched
                    .next()
                    .ok_or_else(|| format!("no node for role {role:?}, label {label:?}"))?;
                if matched.next().is_some() {
                    return Err(format!("multiple nodes for role {role:?}, label {label:?}"));
                }
                return Ok(PluginValue::F64(node.id.0 as f64));
            }
            "back" => {
                let event = match text(args, 1)? {
                    "start" => BackGesture::Started {
                        from_right: number(args, 2)? != 0.,
                    },
                    "progress" => BackGesture::Progress(number(args, 2)? as f32),
                    "cancel" => BackGesture::Cancelled,
                    "commit" => BackGesture::Committed,
                    _ => return Err("unknown back gesture phase".to_owned()),
                };
                return Ok(PluginValue::Bool(self.session.back_gesture(event)));
            }
            kind @ ("click" | "wheel" | "drag") => {
                let id = number(args, 1)?;
                let semantics = self.snapshot()?;
                let rect = semantics
                    .nodes
                    .iter()
                    .find(|node| node.id.0 as f64 == id)
                    .ok_or_else(|| format!("no visible node {id}"))?
                    .rect;
                let (x, y) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
                if kind == "wheel" {
                    self.session.pointer(PointerEvent::wheel(
                        x,
                        y,
                        self.now,
                        number(args, 2)? as f32,
                        number(args, 3)? as f32,
                    ));
                } else {
                    let mut down = PointerEvent::new(Phase::Down, x, y, self.now);
                    let mut up = PointerEvent::new(Phase::Up, x, y, self.now + 8.);
                    up.buttons = Buttons::empty();
                    if kind == "drag" {
                        let (dx, dy) = (number(args, 2)? as f32, number(args, 3)? as f32);
                        down.kind = PointerKind::Touch;
                        up.kind = PointerKind::Touch;
                        self.session.pointer(down);
                        let mut movement =
                            PointerEvent::new(Phase::Move, x + dx, y + dy, self.now + 12.);
                        movement.kind = PointerKind::Touch;
                        movement.button = Button::None;
                        self.session.pointer(movement);
                        up.x += dx;
                        up.y += dy;
                        up.time_ms += 12.;
                    } else {
                        self.session.pointer(down);
                    }
                    self.now = up.time_ms;
                    self.session.pointer(up);
                }
            }
            "replace" => {
                let value = text(args, 1)?.to_owned();
                let len = engine::session::utf16_len(&value);
                self.session.input(InputEvent::Replace {
                    text: value,
                    start: len,
                    end: len,
                    compose_start: number(args, 2)? as i32,
                    compose_end: number(args, 3)? as i32,
                });
            }
            "snapshot" => self.snapshot_image(Path::new(text(args, 1)?))?,
            other => return Err(format!("unknown test command {other}")),
        }
        Ok(PluginValue::Null)
    }

    fn snapshot_image(&mut self, name: &Path) -> Result<(), String> {
        #[cfg(feature = "headless-gpu")]
        if let Some(snapshots) = &mut self.snapshots {
            return snapshots
                .compare(name)
                .map_err(|error| format!("{error:#}"));
        }
        Err(format!(
            "snapshot {} requires impasto test --gpu",
            name.display()
        ))
    }

    fn snapshot(&mut self) -> Result<SemanticsUpdate, String> {
        self.session
            .semantics()
            .ok_or_else(|| "the screen has no semantics tree".to_owned())
    }
}

fn role_name(role: SemanticsRole) -> &'static str {
    match role {
        SemanticsRole::Generic => "generic",
        SemanticsRole::Text => "text",
        SemanticsRole::Button => "button",
        SemanticsRole::Image => "image",
        SemanticsRole::Scroll => "scroll",
        SemanticsRole::Switch => "switch",
        SemanticsRole::TextField => "textbox",
        SemanticsRole::SearchField => "searchbox",
        SemanticsRole::TextArea => "textarea",
    }
}

fn trace_values(now: &FrameTrace, before: &FrameTrace) -> PluginValue {
    PluginValue::Map(
        [
            ("createdNodes", now.nodes_created - before.nodes_created),
            ("ffiBatches", now.ffi_batches - before.ffi_batches),
            ("ffiCalls", now.ffi_calls - before.ffi_calls),
            ("styleResolves", now.style_resolves - before.style_resolves),
            (
                "repaintBoundariesDirtied",
                now.repaint_boundaries_dirtied - before.repaint_boundaries_dirtied,
            ),
            ("updates", now.updates - before.updates),
            ("sets", now.sets - before.sets),
            ("layoutPasses", now.layout_passes - before.layout_passes),
            ("textShapings", now.text_shaped - before.text_shaped),
            ("pointers", now.pointers - before.pointers),
            ("edits", now.edits - before.edits),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), PluginValue::F64(f64::from(value))))
        .collect(),
    )
}
