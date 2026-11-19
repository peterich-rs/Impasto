#[allow(clippy::expect_used, reason = "test fixtures and process assertions")]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{Command, Output},
        sync::atomic::{AtomicU64, Ordering},
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
