use std::{collections::BTreeMap, fs, path::Path};

use engine::{
    js::{compile_archive, open_compiled},
    session::{FrameTime, Metrics, ModuleConfig, ModuleRegistry},
    text::MonoShaper,
};

fn framework() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn release_archive_runs_a_lazy_chunk_only_after_dynamic_import() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    fs::write(
        &entry,
        r#"
        import { boot, scene, Text } from "@impasto/solid-renderer";
        import { createSignal } from "solid-js";
        const [label, setLabel] = createSignal("entry");
        boot(() => <Text>{label()}</Text>);
        setTimeout(async () => {
          globalThis.allowed = true;
          const lazy = await import("./lazy.ts");
          setLabel(lazy.value);
        }, 30);
    "#,
    )
    .unwrap();
    fs::write(
        root.join("lazy.ts"),
        r#"
        if (!globalThis.allowed) throw new Error("lazy chunk initialized early");
        export const value = "lazy loaded";
    "#,
    )
    .unwrap();
    let linked = bundler::bundle(bundler::BundleOptions {
        root: root.to_owned(),
        entry,
        framework: framework(),
        release: true,
        split: true,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(linked.chunks.len(), 1);
    let sources = BTreeMap::from_iter(
        std::iter::once(("app.js".to_owned(), linked.code)).chain(
            linked
                .chunks
                .into_iter()
                .map(|(name, chunk)| (name, chunk.code)),
        ),
    );
    let compiled =
        compile_archive("app.js", sources, ModuleRegistry::new(Vec::new()), &[]).unwrap();
    assert_eq!(&compiled.as_bytes()[..4], b"IQJB");
    assert_eq!(compiled.module_sizes().len(), 2);
    let mut session = open_compiled(
        &compiled,
        "app.js",
        MonoShaper::default(),
        Box::new(|_, _| {}),
        ModuleConfig::disconnected(),
    )
    .unwrap();
    session.metrics(&Metrics::new(360., 640., 1.));
    session.frame(FrameTime::at(0.));
    assert!(session.inspector_snapshot().unwrap().contains("entry"));
    session.frame(FrameTime::at(32.));
    session.frame(FrameTime::at(48.));
    let faults = session.take_faults();
    assert!(faults.is_empty(), "{faults:?}");
    assert!(
        session
            .inspector_snapshot()
            .unwrap()
            .contains("lazy loaded")
    );
}

#[test]
fn catalogs_validate_and_release_static_calls_drop_unused_messages() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    fs::create_dir(root.join("locales")).unwrap();
    fs::write(
        root.join("locales/en.json"),
        r#"{"trade":{"title":"Buy {symbol}","unused":"UNUSED_MARKER"},"other":{"title":"OTHER_MARKER"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("locales/zh.json"),
        r#"{"trade":{"title":"买入 {symbol}","unused":"UNUSED_MARKER"},"other":{"title":"OTHER_MARKER"}}"#,
    )
    .unwrap();
    assert!(
        bundler::i18n::declarations(root)
            .unwrap()
            .contains("\"symbol\": string | number")
    );
    fs::write(
        &entry,
        r#"
        import { test, assert, screen } from "impasto:test";
        import { setLanguage, t as message } from "@impasto/i18n";
        import { Text } from "@impasto/solid-ui";
        test("locale updates text", async () => {
          setLanguage("en");
          await screen.mount(() => <Text>{message("trade.title", { symbol: "ABC" })}</Text>);
          const id = await screen.byText("Buy ABC");
          setLanguage("zh-CN");
          await screen.advanceFrames();
          assert.equal(await screen.byText("买入 ABC"), id);
        });
    "#,
    )
    .unwrap();
    let options = || {
        let mut options = bundler::BundleOptions {
            root: root.to_owned(),
            entry: entry.clone(),
            framework: framework(),
            release: true,
            ..Default::default()
        };
        testing::install(&mut options);
        options
    };
    let bundle = bundler::bundle(options()).unwrap();
    assert!(!bundle.code.contains("UNUSED_MARKER"));
    assert_eq!(
        testing::run(&bundle.code, "test.js", Vec::new(), None)
            .unwrap()
            .passed,
        1
    );
    fs::write(
        &entry,
        r#"
        import { test, assert } from "impasto:test";
        import { setLanguage, t } from "@impasto/i18n";
        function label(kind: "title") { return t(`trade.${kind}`, { symbol: "ABC" }); }
        function concatenated(kind: "unused") { return t("trade." + kind); }
        test("dynamic keys use only their namespace", () => {
          setLanguage("en");
          assert.equal(label("title"), "Buy ABC");
          assert.equal(concatenated("unused"), "UNUSED_" + "MARKER");
        });
    "#,
    )
    .unwrap();
    let bundle = bundler::bundle(options()).unwrap();
    assert!(!bundle.code.contains("OTHER_MARKER"));
    assert_eq!(
        testing::run(&bundle.code, "test.js", Vec::new(), None)
            .unwrap()
            .passed,
        1
    );
    fs::write(
        root.join("locales/zh.json"),
        r#"{"trade":{"title":"买入 {ticker}","unused":"unused"},"other":{"title":"OTHER_MARKER"}}"#,
    )
    .unwrap();
    let error = bundler::bundle(options()).unwrap_err().to_string();
    assert!(
        error.contains("zh.json") && error.contains("trade.title"),
        "{error}"
    );
}

#[test]
fn filtered_tests_and_virtual_clock_run_without_wall_clock_waits() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("test.ts");
    fs::write(
        &entry,
        r#"
        import { assert, clock, describe, test } from "impasto:test";
        test("skip me", () => assert.fail());
        describe("matching", () => test("virtual timer", async () => {
          await clock.install();
          const before = performance.now();
          let ticks = 0;
          const interval = setInterval(() => ticks++, 700);
          let fired = false;
          setTimeout(() => { fired = true; }, 2200);
          await clock.advance(2100);
          assert.equal(ticks, 3);
          assert.equal(fired, false);
          assert.equal(performance.now() - before, 2100);
          await clock.advance(100);
          clearInterval(interval);
          assert.equal(fired, true);
        }));
    "#,
    )
    .unwrap();
    let mut options = bundler::BundleOptions {
        root: root.to_owned(),
        entry,
        framework: framework(),
        ..Default::default()
    };
    testing::install(&mut options);
    let bundle = bundler::bundle(options).unwrap();
    let report = testing::run_filtered(
        &bundle.code,
        "test.js",
        Vec::new(),
        None,
        None,
        Some("matching / virtual".to_owned()),
    )
    .unwrap();
    assert_eq!(report.passed, 1);
}
