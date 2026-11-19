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
    #[test]
    fn external_app_builds_checks_and_runs_without_a_window() {
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !framework
            .join("node_modules/typescript/package.json")
            .is_file()
        {
            eprintln!("CLI compiler integration requires pnpm install; skipping without JS tools");
            return;
        }
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
        assert_eq!(names, ["app.toml", "assets", "lib", "package.json"]);
        success(sandbox.cli(&["analyze"]));
        fs::write(sandbox.0.join("lib/probe.test.ts"),"import {createSignal,batch} from 'solid-js'; const [get,set]=createSignal(1); batch(()=>set(2)); if(get()!==2)throw new Error('signal');").expect("test source");
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
                sandbox.0.join("lib/probe.test.ts"),
                "throw new Error('intentional failure');",
            )
            .expect("failure test");
            let failed = sandbox.cli(&["test"]);
            assert!(!failed.status.success());
            assert!(String::from_utf8_lossy(&failed.stderr).contains("intentional failure"));
        }
        let ios = sandbox.cli(&["build", "ios"]);
        assert!(!ios.status.success());
        assert!(String::from_utf8_lossy(&ios.stderr).contains("crates/ios"));
        success(sandbox.cli(&["clean"]));
        assert!(!sandbox.0.join("build").exists());
        assert!(sandbox.0.join("lib/main.tsx").is_file());
        assert!(sandbox.0.join("app.toml").is_file());
    }
    #[test]
    fn native_capabilities_typecheck_and_link_from_the_public_packages() {
        let framework = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !framework
            .join("node_modules/typescript/package.json")
            .is_file()
        {
            eprintln!("CLI compiler integration requires pnpm install; skipping without JS tools");
            return;
        }
        let mut sandbox = Sandbox::new();
        success(sandbox.cli(&["create", "capabilities"]));
        sandbox.0.push("capabilities");
        fs::write(
            sandbox.0.join("lib/main.tsx"),
            r#"
import { AppState } from "@impasto/app-state";
import { AsyncStorage } from "@impasto/async-storage";
import { Clipboard } from "@impasto/clipboard";
import { store } from "@impasto/devtools";
import { FileSystem } from "@impasto/file-system";
import { Linking } from "@impasto/linking";
import { Permission } from "@impasto/permissions";
import { boot } from "@impasto/solid-renderer";

boot(() => <view />);
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
        success(sandbox.cli(&["analyze"]));
        fs::write(sandbox.0.join("16.bin"), [65; 16]).expect("small file");
        fs::write(sandbox.0.join("8192.bin"), [65; 8192]).expect("large file");
        for debug in [true, false] {
            let mut args = vec!["build", "bundle"];
            if debug {
                args.push("--debug");
            }
            success(sandbox.cli(&args));
            let (tx, rx) = mpsc::channel();
            let storage = shell::memory::MemoryStorage::new(sandbox.0.to_string_lossy());
            let mut modules = shell::first_party_modules();
            modules.retain(|module| !matches!(module.name, "FileSystem" | "AsyncStorage"));
            modules.push(shell::file_system(Arc::clone(&storage) as _));
            modules.push(shell::async_storage(storage));
            let config = session::ModuleConfig {
                registry: Arc::new(session::ModuleRegistry::new(modules)),
                post_delivery: None,
                settle: Arc::new(move |token| {
                    let tx = tx.clone();
                    session::Settle::new(move |result| {
                        let _ = tx.send(session::IoEvent::Plugin { token, result });
                    })
                }),
                ..session::ModuleConfig::disconnected()
            };
            let completed = Arc::new(AtomicBool::new(false));
            let log: quickjs::ConsoleLog = Box::new({
                let completed = Arc::clone(&completed);
                move |_, line| {
                    if line == "capabilities passed" {
                        completed.store(true, Ordering::Relaxed);
                    }
                }
            });
            let shaper = uicore::MonoShaper::default();
            let mut session = if debug {
                let source = fs::read(sandbox.0.join("build/app.js")).expect("ESM bundle");
                quickjs::open_program_with_io(&source, "app.js", shaper, log, None, config)
            } else {
                let source = fs::read(sandbox.0.join("build/app.bin")).expect("bytecode bundle");
                // SAFETY: this test loads only the artifact built above by the pinned CLI.
                #[expect(unsafe_code, reason = "loads this test's own compiled artifact")]
                unsafe {
                    quickjs::open_trusted_bytecode_with_io(
                        &source, "app.bin", shaper, log, None, config,
                    )
                }
            }
            .expect("link capability imports");
            session.metrics(&session::Metrics::new(360.0, 640.0, 1.0));
            session.set_launch_url("demo://open");
            session.lifecycle(session::Lifecycle::Inactive);
            session.window_focus(false);
            for step in 0..8 {
                for event in rx.try_iter() {
                    session.io_event(event);
                }
                session.frame(session::FrameTime::at(f64::from(step) * 16.0));
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
}
