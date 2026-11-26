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
    fs::write(root.join("icon.svg"), r#"<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="M0 0L12 0L12 12Z"/></svg>"#).unwrap();
    fs::write(
        &entry,
        r#"
      import { assert, screen, test } from "impasto:test";
      import { scene } from "@impasto/solid-renderer";
      import icon from "./icon.svg";
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
          const binding = scene.bindCanvas(node, ctx => { icon.draw(ctx); draws++; });
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
    let report = testing::run(&bundle.code, "test.js", Vec::new(), None).unwrap();
    assert_eq!(report.passed, 1);
}
