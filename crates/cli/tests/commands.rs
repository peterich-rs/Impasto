#[allow(clippy::expect_used, reason = "test fixtures and process assertions")]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{Command, Output},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
            mpsc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };
    static SANDBOX_SEQ: AtomicU64 = AtomicU64::new(0);
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let seq = SANDBOX_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("impasto-cli-{}-{stamp}-{seq}", std::process::id()));
            fs::create_dir(&dir).expect("sandbox");
            Self(dir)
        }
        fn cli(&self, args: &[&str]) -> Output {
            Command::new(env!("CARGO_BIN_EXE_impasto"))
                .args(args)
                .current_dir(&self.0)
                .output()
                .expect("impasto")
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn success(output: Output) {
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    struct ReadyExec;

    impl impasto::abi::MethodExec for ReadyExec {
        fn spawn(&self, task: impasto::abi::HostFuture) {
            pollster::block_on(task);
        }
        fn worker(&self, settle: impasto::abi::Settle, work: impasto::abi::Work) {
            settle.send(work());
        }
        fn main_thread(&self, settle: impasto::abi::Settle, work: impasto::abi::Work) {
            settle.send(work());
        }
        fn now(&self, settle: impasto::abi::Settle, result: impasto::abi::PluginResult) {
            settle.send(result);
        }
    }

    #[test]
    fn standalone_pack_resolves_an_explicit_project_and_validates_defines() {
        let sandbox = Sandbox::new();
        let project = sandbox.0.join("external");
        fs::create_dir(&project).expect("project directory");
        fs::write(project.join("main.js"), "console.log(REPLAY_VERSION);").expect("entry");
        success(sandbox.cli(&[
            "--project",
            "external",
            "pack",
            "main.js",
            "build/replay.js",
            "--define",
            "REPLAY_VERSION='seeded-replay'",
        ]));
        let bundle = fs::read_to_string(project.join("build/replay.js")).expect("bundle");
        assert!(bundle.contains("seeded-replay"));
        assert!(!bundle.contains("REPLAY_VERSION"));
        let failed = sandbox.cli(&[
            "--project",
            "external",
            "pack",
            "main.js",
            "build/invalid.js",
            "--define",
            "REPLAY_VERSION=1",
            "--define",
            "REPLAY_VERSION=2",
        ]);
        assert!(!failed.status.success());
        assert!(String::from_utf8_lossy(&failed.stderr).contains("REPLAY_VERSION"));
        assert!(!project.join("build/invalid.js").exists());
    }

    #[test]
    fn external_app_builds_checks_and_runs_without_a_window() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "sample"]));
        sandbox.0.push("sample");
        let mut names = fs::read_dir(&sandbox.0)
            .expect("app")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                ".gitignore",
                ".impasto",
                "android",
                "app.toml",
                "assets",
                "ios",
                "macos",
                "src",
                "tsconfig.json"
            ]
        );
        success(sandbox.cli(&["analyze"]));
        fs::write(sandbox.0.join("src/probe.test.ts"),"import {createSignal,batch} from 'solid-js'; const [get,set]=createSignal(1); batch(()=>set(2)); if(get()!==2)throw new Error('signal');").expect("test source");
        success(sandbox.cli(&["build", "bundle", "--debug"]));
        assert!(sandbox.0.join("build/app.js").is_file());
        let native = sandbox.cli(&["test"]);
        let linked = !String::from_utf8_lossy(&native.stderr).contains("not linked");
        if linked {
            success(native);
            success(sandbox.cli(&["build", "bundle"]));
            assert!(sandbox.0.join("build/app.bin").is_file());
            assert!(!sandbox.0.join("build/app.js").exists());
            success(sandbox.cli(&["test", "build/app.bin"]));
            fs::write(
                sandbox.0.join("src/probe.test.ts"),
                "throw new Error('intentional failure');",
            )
            .expect("failure test");
            let failed = sandbox.cli(&["test"]);
            assert!(!failed.status.success());
            assert!(String::from_utf8_lossy(&failed.stderr).contains("intentional failure"));
        }
        let ios = sandbox.cli(&["build", "ios"]);
        assert!(!ios.status.success());
        let reason = if cfg!(target_os = "macos") {
            "ios.development_team"
        } else {
            "macOS 与 Xcode"
        };
        assert!(String::from_utf8_lossy(&ios.stderr).contains(reason));
        success(sandbox.cli(&["clean"]));
        assert!(!sandbox.0.join("build").exists());
        assert!(sandbox.0.join("src/main.tsx").is_file());
        assert!(sandbox.0.join("app.toml").is_file());
    }
    #[test]
    fn native_ui_test_runs_real_pointer_hit_testing_and_layout() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "counter"]));
        sandbox.0.push("counter");
        fs::write(
            sandbox.0.join("src/counter.test.tsx"),
            r#"
import { test, expect, screen, fire, trace } from "impasto:test";
import { Button, Text } from "@impasto/solid-ui";
import { View } from "@impasto/solid-renderer/host";
import { createSignal } from "solid-js";

function Counter() {
    const [count, setCount] = createSignal(0);
    return <View style={{ padding: 12 }}>
        <Button accessibilityLabel="increment" onClick={() => setCount(count() + 1)}>
            <Text>Increment</Text>
        </Button>
        <Text>{`Count ${count()}`}</Text>
    </View>;
}

test("click re-renders text", async () => {
    await screen.mount(() => <Counter />);
    const button = await screen.byLabel("increment");
    expect(screen.rect(button).w).toBeGreaterThan(5);
    const stats = await trace(() => fire.click(button));
    expect(stats.pointers).toBe(2);
    expect(await screen.byText("Count 1")).toBeVisible();
});
"#,
        )
        .expect("write test source");
        success(sandbox.cli(&["test", "--trace"]));
    }

    #[test]
    fn native_capabilities_typecheck_and_link_from_the_public_packages() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "capabilities"]));
        sandbox.0.push("capabilities");
        fs::write(
            sandbox.0.join("src/main.tsx"),
            r#"
import { AppState } from "@impasto/app-state";
import { AsyncStorage } from "@impasto/async-storage";
import { Clipboard } from "@impasto/clipboard";
import { store } from "@impasto/devtools";
import { FileSystem } from "@impasto/file-system";
import { Linking } from "@impasto/linking";
import { Permission } from "@impasto/permissions";
import { boot } from "@impasto/solid-renderer";
import { View } from "@impasto/solid-ui";

boot(() => <View />);
Clipboard.setString("copied");
const copied: string = Clipboard.getString();
if (copied !== "copied" || AppState.currentState !== "active" || !store.available()) {
  throw new Error("sync capability exports");
}
for (const plugin of [Clipboard, AsyncStorage, FileSystem]) {
  if ("addEventListener" in plugin || "removeEventListener" in plugin) {
    throw new Error("listener exposed on a module without events");
  }
}
let receivedURL: string | undefined;
let receivedState: string | undefined;
let receivedFocus: "true" | "false" | undefined;
let removedCalls = 0;
const removedListener = () => { removedCalls += 1; };
Linking.addEventListener("url", removedListener);
Linking.removeEventListener("url", removedListener);
Linking.addEventListener("url", ({ url }) => { receivedURL = url; });
AppState.addEventListener("change", (state) => { receivedState = state; });
AppState.addEventListener("focus", (focus) => { receivedFocus = focus; });
const initialURL: string | null = Linking.getInitialURL();
if (initialURL !== null) throw new Error("initial URL");

async function check() {
  const [[key, value]] = await AsyncStorage.multiGet(["missing"]);
  if (key !== "missing" || value !== null) throw new Error("storage tuple");
  if (receivedURL !== "demo://open" || receivedState !== "inactive" ||
      receivedFocus !== "false" || removedCalls !== 0) {
    throw new Error("declared module events");
  }
  if ((await Permission.camera.status) !== "denied") throw new Error("permission status");
  for (const size of [16, 8192]) {
    const bytes: Uint8Array = await FileSystem.read(`${size}.bin`);
    if (!(bytes instanceof Uint8Array) || bytes.length !== size || bytes[0] !== 65) {
      throw new Error(`file bytes ${size}`);
    }
  }
  console.log("capabilities passed");
}
void check();
"#,
        )
        .expect("capabilities app");
        let package = sandbox.0.join("package.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(&package).unwrap_or_else(|_| b"{\"dependencies\":{}}".to_vec()),
        )
        .expect("JSON");
        for name in [
            "app-state",
            "async-storage",
            "clipboard",
            "devtools",
            "file-system",
            "linking",
            "permissions",
        ] {
            manifest["dependencies"][format!("@impasto/{name}")] = "0.1.0".into();
        }
        fs::write(
            package,
            serde_json::to_vec(&manifest).expect("package JSON"),
        )
        .expect("dependencies");
        success(sandbox.cli(&["analyze", "--platform", "macos"]));
        fs::write(sandbox.0.join("16.bin"), [65; 16]).expect("small file");
        fs::write(sandbox.0.join("8192.bin"), [65; 8192]).expect("large file");
        for debug in [true, false] {
            let mut args = vec!["build", "bundle", "--platform", "macos"];
            if debug {
                args.push("--debug");
            }
            success(sandbox.cli(&args));
            let (tx, rx) = mpsc::channel();
            let mut registrar = impasto::PluginRegistrar::default();
            impasto_bundled_plugins::register(&mut registrar);
            let publish = tx.clone();
            let context = impasto::PluginContext::new(
                impasto::Runtime::default(),
                impasto::AppPaths::new(Some(sandbox.0.clone()), None, None),
                move |event| publish.send(engine::session::IoEvent::Host(event)).is_ok(),
            );
            context.insert_extension(impasto::DiagnosticsSource::new(|_, _| {
                Ok(impasto::abi::PluginValue::Null)
            }));
            let modules = registrar.build(&context).expect("capability registry");
            let config = engine::session::ModuleConfig {
                registry: Arc::new(engine::session::ModuleRegistry::new(modules)),
                post_delivery: None,
                exec: Arc::new(ReadyExec),
                settle: Arc::new(move |token| {
                    let tx = tx.clone();
                    engine::session::Settle::new(move |result| {
                        let _ = tx.send(engine::session::IoEvent::Plugin { token, result });
                    })
                }),
                ..engine::session::ModuleConfig::disconnected()
            };
            let completed = Arc::new(AtomicBool::new(false));
            let log: engine::js::ConsoleLog = Box::new({
                let completed = Arc::clone(&completed);
                move |_, line| {
                    if line == "capabilities passed" {
                        completed.store(true, Ordering::Relaxed);
                    }
                }
            });
            let shaper = engine::text::MonoShaper::default();
            let mut session = if debug {
                let source = fs::read(sandbox.0.join("build/macos/bundle/debug/app.js"))
                    .expect("ESM bundle");
                engine::js::open_program(&source, "app.js", shaper, log, config)
            } else {
                let source = fs::read(sandbox.0.join("build/macos/bundle/release/app.bin"))
                    .expect("bytecode bundle");
                // SAFETY: this test loads only the artifact built above by the pinned CLI.
                #[expect(unsafe_code, reason = "loads this test's own compiled artifact")]
                unsafe {
                    engine::js::open_trusted_bytecode(&source, "app.bin", shaper, log, config)
                }
            }
            .expect("link capability imports");
            session.metrics(&engine::session::Metrics::new(360.0, 640.0, 1.0));
            context.dispatch_open_url("demo://open");
            context.dispatch_lifecycle(impasto::Lifecycle::Inactive);
            context.dispatch_window_focus(true);
            context.dispatch_window_focus(false);
            for step in 0..8 {
                for event in rx.try_iter() {
                    session.io_event(event);
                }
                session.frame(engine::session::FrameTime::at(f64::from(step) * 16.0));
            }
            let faults = session.take_faults();
            assert!(faults.is_empty(), "{faults:?}");
            assert!(
                completed.load(Ordering::Relaxed),
                "capability calls did not finish"
            );
        }
    }
    #[test]
    fn invalid_names_and_manifests_fail_before_writing_artifacts() {
        let sandbox = Sandbox::new();
        assert!(!sandbox.cli(&["create", "../escape"]).status.success());
        assert!(!sandbox.cli(&["create", "bad\"name"]).status.success());
        success(sandbox.cli(&["create", "valid"]));
        fs::write(
            sandbox.0.join("valid/app.toml"),
            "name = 'valid'\nversion = '0.1.0'\ntargets = ['windows']\n",
        )
        .expect("manifest");
        let result = Command::new(env!("CARGO_BIN_EXE_impasto"))
            .args(["build", "bundle"])
            .current_dir(sandbox.0.join("valid"))
            .output()
            .expect("invalid build");
        assert!(!result.status.success());
        assert!(!sandbox.0.join("valid/build").exists());
    }
    #[test]
    fn sync_preserves_platform_sources_and_keeps_unsupported_plugins_disabled() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "managed", "--with-rust"]));
        sandbox.0.push("managed");
        assert!(sandbox.0.join("rust/Cargo.toml").is_file());
        let activity = sandbox
            .0
            .join("android/app/src/main/kotlin/dev/impasto/runner/MainActivity.kt");
        let customized = fs::read_to_string(&activity).expect("Android entry")
            + "\n// Application customization\n";
        fs::write(&activity, &customized).expect("custom activity");
        let package = sandbox.0.join("package.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(&package).unwrap_or_else(|_| b"{\"dependencies\":{}}".to_vec()),
        )
        .expect("JSON");
        manifest["dependencies"]["@impasto/camera"] = "0.1.0".into();
        fs::write(
            package,
            serde_json::to_vec(&manifest).expect("package JSON"),
        )
        .expect("dependencies");

        success(sandbox.cli(&["sync", "--platform", "macos"]));
        let generated = sandbox.0.join(".impasto/plugins-manifest.macos.json");
        let first = fs::read(&generated).expect("macOS manifest");
        success(sandbox.cli(&["sync", "--platform", "macos"]));
        assert_eq!(fs::read(&generated).expect("synced manifest"), first);
        assert_eq!(
            fs::read_to_string(activity).expect("custom entry"),
            customized
        );
        let contents: serde_json::Value = serde_json::from_slice(&first).expect("plugin manifest");
        assert_eq!(
            contents["disabledEntries"],
            serde_json::json!(["@impasto/camera"])
        );
        assert_eq!(contents["modules"][0]["specifier"], "@app/managed/native");
        let entry = fs::read_to_string(sandbox.0.join(".impasto/cargo/entry-macos/Cargo.toml"))
            .expect("generated macOS crate");
        assert!(entry.contains("default-features = false"));

        let stale = sandbox.0.join(".impasto/cargo/registry-ios/Cargo.toml");
        let registry = fs::read_to_string(&stale).expect("iOS registry");
        fs::write(
            &stale,
            registry + "\nremoved = { path = \"/impasto-test-removed-plugin\" }\n",
        )
        .expect("stale dependency");
        success(sandbox.cli(&["sync", "--platform", "macos"]));
        let workspace = fs::read_to_string(sandbox.0.join(".impasto/cargo/Cargo.toml"))
            .expect("generated workspace");
        assert!(!workspace.contains("registry-ios"));
    }

    #[test]
    fn add_bundled_package_updates_source_and_cargo_manifests() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "withhttp"]));
        sandbox.0.push("withhttp");
        success(sandbox.cli(&["add", "@impasto/http", "--rust"]));
        let manifest = fs::read_to_string(sandbox.0.join("app.toml")).expect("app manifest");
        assert!(manifest.contains("@impasto/http"));
        let manifest =
            fs::read_to_string(sandbox.0.join("rust/Cargo.toml")).expect("Cargo manifest");
        assert!(manifest.contains("impasto-http") && manifest.contains("registry = \"impasto\""));
    }

    #[test]
    fn local_registry_resolves_rust_plugins_offline_and_initializes_dependencies_first() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "closure"]));
        sandbox.0.push("closure");
        let plugins = sandbox.0.join("plugins");
        let base = plugins.join("base");
        let consumer = plugins.join("consumer");
        for (root, name, extra, package_deps) in [
            (&base, "z-base", "", serde_json::json!({})),
            (
                &consumer,
                "a-consumer",
                "fixture-z-base = { version = \"0.1.0\", registry = \"impasto\" }",
                serde_json::json!({"@fixture/z-base": "file:../base"}),
            ),
        ] {
            fs::create_dir_all(root.join("rust/src")).expect("plugin sources");
            fs::create_dir_all(root.join("src")).expect("plugin JS");
            fs::write(root.join("src/index.ts"), "export {};").expect("JS entry");
            fs::write(
                root.join("plugin.toml"),
                "[plugin]\nplatforms = [\"macos\"]\n[rust]\npath = \"rust\"\n",
            )
            .expect("plugin manifest");
            fs::write(
                root.join("package.json"),
                serde_json::json!({
                    "name": format!("@fixture/{name}"),
                    "version": "0.1.0",
                    "exports": { ".": "./src/index.ts" },
                    "dependencies": package_deps,
                })
                .to_string(),
            )
            .expect("npm manifest");
            fs::write(root.join("rust/Cargo.toml"), format!(
                "[package]\nname = \"fixture-{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nimpasto = {{ version = \"0.1.0\", registry = \"impasto\" }}\n{extra}\n"
            )).expect("Cargo manifest");
        }
        fs::write(
            base.join("rust/src/lib.rs"),
            r#"
pub struct Value(pub u32);
#[impasto::module(specifier = "@fixture/z-base/native", init = init)]
struct Provider;
impl Provider {
    fn init(ctx: &impasto::PluginContext) -> Result<Self, impasto::PluginInitError> {
        ctx.insert_extension(Value(42));
        Ok(Self)
    }
}
#[impasto::export]
impl Provider {
    pub fn ready() -> bool { true }
}
pub fn register(registrar: &mut impasto::PluginRegistrar) {
    registrar.add(Provider::plugin());
}
"#,
        )
        .expect("provider source");
        fs::write(
            consumer.join("rust/src/lib.rs"),
            r#"
#[impasto::module(specifier = "@fixture/a-consumer/native", init = init)]
struct Consumer(u32);
impl Consumer {
    fn init(ctx: &impasto::PluginContext) -> Result<Self, impasto::PluginInitError> {
        let value = ctx.extension::<fixture_z_base::Value>()
            .ok_or_else(|| impasto::PluginInitError::new("base plugin must initialize first"))?;
        Ok(Self(value.0))
    }
}
#[impasto::export]
impl Consumer {
    pub fn answer(&self) -> u32 { self.0 }
}
pub fn register(registrar: &mut impasto::PluginRegistrar) {
    registrar.add(Consumer::plugin());
}
"#,
        )
        .expect("consumer source");
        let app_manifest = sandbox.0.join("app.toml");
        let mut app = fs::read_to_string(&app_manifest).expect("app manifest");
        app.push_str(
            r#"

[dependencies]
"@fixture/a-consumer" = { path = "./plugins/consumer" }
"@fixture/z-base" = { path = "./plugins/base" }
"#,
        );
        fs::write(app_manifest, app).expect("app dependencies");
        let sync = || {
            Command::new(env!("CARGO_BIN_EXE_impasto"))
                .args(["sync", "--platform", "macos"])
                .current_dir(&sandbox.0)
                .env("CARGO_NET_OFFLINE", "true")
                .output()
                .expect("offline impasto sync")
        };
        success(sync());
        let workspace = sandbox.0.join(".impasto/cargo");
        let tests = workspace.join("registry-macos/tests");
        fs::create_dir(&tests).expect("registry tests");
        fs::write(
            tests.join("registry_order.rs"),
            r#"
#[test]
fn builds_dependency_first() {
    let mut registrar = impasto::PluginRegistrar::default();
    impasto_registry_macos::register(&mut registrar);
    let modules = registrar.build(&impasto::PluginContext::default()).expect("build modules");
    let registry = impasto::abi::ModuleRegistry::new(modules);
    assert_eq!(registry.call("Consumer", "answer", &[]), Ok(impasto::abi::PluginValue::F64(42.0)));
}
"#,
        )
        .expect("registry test");
        success(
            Command::new("cargo")
                .current_dir(&workspace)
                .args([
                    "test",
                    "--offline",
                    "-p",
                    "impasto-registry-macos",
                    "--test",
                    "registry_order",
                ])
                .output()
                .expect("run local registry"),
        );
        let package_path = consumer.join("package.json");
        let mut package: serde_json::Value =
            serde_json::from_slice(&fs::read(&package_path).expect("plugin manifest"))
                .expect("JSON");
        package["dependencies"] = serde_json::json!({});
        fs::write(package_path, package.to_string()).expect("remove npm dependency");
        let result = sync();
        assert!(!result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("fixture-a-consumer")
                && stderr.contains("fixture-z-base")
                && stderr.contains("package.json"),
            "{stderr}"
        );
    }

    #[test]
    fn ios_manifest_and_options_are_validated_before_staging() {
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "iosprobe"]));
        sandbox.0.push("iosprobe");
        let manifest = sandbox.0.join("app.toml");
        let base = fs::read_to_string(&manifest).expect("default manifest");
        assert!(base.contains("\"ios\""));

        let non_ios = sandbox.cli(&["build", "bundle", "--ios-simulator"]);
        assert!(!non_ios.status.success());
        assert!(String::from_utf8_lossy(&non_ios.stderr).contains("--ios-simulator"));
        for (config, field) in [
            ("bundle_id = 'dev impasto.app'", "bundle_id"),
            ("development_team = 'wrong'", "development_team"),
            ("build_number = 0", "build_number"),
            ("url_schemes = ['bad scheme']", "url_schemes"),
            (
                "associated_domains = ['applinks:https://bad.test']",
                "associated_domains",
            ),
            ("device_families = [1, 1]", "device_families"),
            (
                "signing_certificate = 'Apple Development'",
                "signing_certificate",
            ),
            (
                "usage_descriptions = { NSCameraUsageDescription = '' }",
                "usage_descriptions",
            ),
            ("privacy_manifest = 'manifest.xml'", "privacy_manifest"),
        ] {
            fs::write(&manifest, format!("{base}\n[ios]\n{config}\n")).expect("iOS manifest");
            let failed = sandbox.cli(&["build", "ios", "--ios-simulator"]);
            assert!(!failed.status.success());
            assert!(String::from_utf8_lossy(&failed.stderr).contains(&format!("ios.{field}")));
        }
        fs::write(&manifest, &base).expect("restore valid manifest");
        for (args, message) in [
            (&["build", "ios-archive", "--debug"][..], "release"),
            (
                &["build", "ios", "--export-options", "settings.plist"][..],
                "build ipa",
            ),
            (&["build", "ipa"][..], "--export-options"),
            (&["run", "-d", "ios", "--watch", "--release"][..], "--watch"),
        ] {
            let failed = sandbox.cli(args);
            assert!(!failed.status.success());
            assert!(String::from_utf8_lossy(&failed.stderr).contains(message));
        }
        assert!(!sandbox.0.join("build").exists());
    }
}
