use std::{fs, path::Path};

#[test]
fn links_shader_uniforms_and_draws_build_time_svg_paths() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let entry = root.join("test.ts");
    fs::write(
        root.join("effect.wgsl"),
        r#"
      struct EffectUniforms {
        opacity: f32,
        tint: vec3<f32>,
      }
      fn effect_main(position: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
        return vec4(effect_uniform(4u), effect_uniform(5u), effect_uniform(6u), effect_uniform(0u));
      }
    "#,
    )
    .unwrap();
    fs::write(root.join("icon.svg"), r##"
      <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
        <defs><linearGradient id="paint"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs>
        <path transform="translate(2 3)" color="#0197e3" fill="currentColor" d="M0 0L12 0L12 12Z"/>
        <path fill="currentColor" d="M17 0L19 0L19 2Z"/>
        <path fill="currentColor" d="M5 5L8 5L8 8Z"/>
        <path fill="currentColor" d="M6 5L9 5L9 8Z"/>
        <path fill="url(#paint)" d="M20 0L24 0L24 12Z"/>
      </svg>
    "##).unwrap();
    fs::write(
        root.join("stroke.svg"),
        r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" fill="none" stroke="currentColor" stroke-linecap="round">
          <path d="M2 2L9 9"/><path d="M18 6L21 9"/>
        </svg>"#,
    )
    .unwrap();
    fs::write(
        &entry,
        r#"
      import { assert, screen, test } from "impasto:test";
      import { scene } from "@impasto/solid-renderer";
      import icon from "./icon.svg";
      import stroke from "./stroke.svg";
      import shader, { uniformIndex, packUniforms } from "./effect.wgsl";

      test("uses generated Canvas paths and f32 slots", async () => {
        const packed = packUniforms({ opacity: 0.5, tint: [1, 0.5, 0] });
        const values = new DataView(packed.buffer);
        assert.equal(uniformIndex.tint, 4);
        assert.equal(values.getFloat32(0, true), 0.5);
        assert.equal(values.getFloat32(16, true), 1);
        assert.match(shader, /effect_main/);
        assert.equal(icon.width, 24);
        let draws = 0;
        await screen.mount(() => {
          const node = scene.createElement("view", { style: { width: 24, height: 24 } });
          scene.bindCanvas(node, ctx => {
            icon.draw(ctx, { color: [255, 0, 0, 255] });
            stroke.draw(ctx, { color: "red", opacity: 0.5 });
            draws++;
          });
          return node;
        });
        assert.ok(draws > 0);
      });
    "#,
    )
    .unwrap();
    let mut options = bundler::BundleOptions {
        root: root.to_path_buf(),
        entry,
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        ..Default::default()
    };
    testing::install(&mut options);
    let bundle = bundler::bundle(options).unwrap();
    assert!(bundle.code.contains("\"M2 3L14 3L14 15Z\""));
    assert!(bundle.code.contains("[\"#0197e3\"]"));
    assert!(bundle.code.contains("\"M17 0L19 0L19 2ZM5 5L8 5L8 8Z\""));
    assert!(bundle.code.contains("\"M6 5L9 5L9 8Z\""));
    assert!(bundle.code.contains("createLinearGradient"));
    assert!(bundle.code.contains("strokeAsset(24, 24"));
    let report = testing::run(&bundle.code, "test.js", Vec::new(), None).unwrap();
    assert_eq!(report.passed, 1);
}

#[test]
fn imported_fonts_use_the_packaged_asset_reader() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let framework = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entry = root.join("test.ts");
    let assets = root.join("assets");
    fs::create_dir_all(assets.join("fonts")).unwrap();
    fs::copy(
        framework.join("crates/embedder/android/assets/fonts/Sans.ttf"),
        assets.join("fonts/Sans.ttf"),
    )
    .unwrap();
    fs::write(
        &entry,
        r#"
      import { assert, test } from "impasto:test";
      import { scene } from "@impasto/solid-renderer";
      import font from "./assets/fonts/Sans.ttf";
      test("uses a native font asset", () => {
        assert.ok(font.family.length > 0);
        assert.ok(font.weight > 0);
        assert.throws(() => scene.registerFontAsset(font.family, "asset:../Sans.ttf"));
      });
    "#,
    )
    .unwrap();
    let mut options = bundler::BundleOptions {
        root: root.to_path_buf(),
        entry,
        framework,
        ..Default::default()
    };
    testing::install(&mut options);
    let bundle = bundler::bundle(options).unwrap();
    assert!(bundle.code.contains("asset:fonts/Sans.ttf"));
    assert!(bundle.code.len() < 100_000, "font bytes were embedded");
    let report =
        testing::run_with_assets(&bundle.code, "test.js", Vec::new(), None, Some(assets)).unwrap();
    assert_eq!(report.passed, 1);
}

#[test]
fn linked_constants_and_templates_are_shared_and_lazy() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let entry = root.join("main.tsx");
    fs::write(
        root.join("palette.ts"),
        r##"
        export const palette = { color: { accent: { ink: "#224466" } } } as const;
        export const spacing = 4;
    "##,
    )
    .unwrap();
    fs::create_dir(root.join("theme")).unwrap();
    fs::write(
        root.join("theme/index.ts"),
        "export { palette, spacing } from '../palette';",
    )
    .unwrap();
    for name in ["one", "two"] {
        fs::write(
            root.join(format!("{name}.tsx")),
            r#"
          import { Column, Text } from "@impasto/solid-ui";
          import { palette, spacing } from "./theme";
          export const render = () => <Column style={{ padding: spacing }}>
            <Text style={{ color: palette.color.accent.ink }}>Shared</Text>
          </Column>;
        "#,
        )
        .unwrap();
    }
    fs::write(
        &entry,
        r#"
      import { render as one } from "./one.tsx";
      import { render as two } from "./two.tsx";
      console.log(one, two);
    "#,
    )
    .unwrap();
    let bundle = bundler::bundle(bundler::BundleOptions {
        root: root.to_path_buf(),
        entry,
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        bundle.code.matches("impasto:pool/template/").count(),
        1,
        "{}",
        bundle.code
    );
    assert_eq!(bundle.code.matches("impasto:pool/style/").count(), 2);
    assert!(!bundle.code.contains("patchInlineStyle"));
}

#[test]
fn browser_global_guard_maps_plain_javascript_and_allows_local_bindings() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let entry = root.join("main.ts");
    let dep = root.join("dependency.js");
    fs::write(
        &entry,
        "import { read } from './dependency.js'; console.log(read());",
    )
    .unwrap();
    let options = || bundler::BundleOptions {
        root: root.to_path_buf(),
        entry: entry.clone(),
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        ..Default::default()
    };
    fs::write(
        &dep,
        "\nexport function read() { return globalThis[\"localStorage\"]; }",
    )
    .unwrap();
    let error = bundler::bundle(options()).unwrap_err().to_string();
    assert!(
        error.contains("dependency.js:2:") && error.contains("localStorage"),
        "{error}"
    );
    fs::write(
        &dep,
        "export function read() { const localStorage = 1; return typeof window + localStorage; }",
    )
    .unwrap();
    assert!(bundler::bundle(options()).is_ok());
}
