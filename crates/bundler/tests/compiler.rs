use std::path::Path;

use bundler::compile_module;

#[test]
fn rejects_lowercase_tags_with_an_import_hint() {
    let result = compile_module(Path::new("app.tsx"), "export const root = <view />;");
    assert!(
        matches!(result, Err(error) if error.contains("Import View explicitly") && error.contains("app.tsx"))
    );
}

#[test]
fn preserves_shadowed_components_and_hoists_known_host_subtrees() -> Result<(), String> {
    let script = r#"
import { View as Panel, Stack } from "@impasto/solid-renderer/host";
enum Phase { Running = 2, Stopped }
export const phase = Phase.Stopped;
export const template = () => <Panel style={{ padding: 8 }}><Stack class="flex"><Panel /></Stack></Panel>;
export function custom(Panel: (props: { title: string }) => number) {
  return <Panel title="User component" />;
}
"#;
    let output = compile_module(Path::new("app.tsx"), script)?;
    assert!(!output.code.contains("enum Phase"));
    assert!(output.code.contains("instantiateTemplate"));
    assert!(output.code.matches("\"1:").count() >= 2);
    assert!(output.code.contains("createComponent(Panel"));
    assert!(
        output
            .map
            .get_sources()
            .any(|file| file.ends_with("app.tsx"))
    );
    Ok(())
}

#[test]
fn diagnoses_invalid_static_host_styles() {
    let script = r#"
import { View } from "@impasto/solid-renderer/host";
export const Main = () => <View style={{ width: "42nonsense" }} />;
"#;
    assert!(
        matches!(compile_module(Path::new("app.tsx"), script), Err(error) if error.contains("width"))
    );
}

#[cfg(feature = "wgsl")]
#[test]
fn validates_literal_custom_effect_arguments() {
    let script = r#"
import { createShaderProgram as effect } from "@impasto/solid-ui";
export const bad = effect("fn wrong() {}");
"#;
    assert!(
        matches!(compile_module(Path::new("app.tsx"), script), Err(error) if error.contains("missing effect_main"))
    );
}

#[test]
fn diagnoses_styles_owned_by_the_virtual_list_driver() {
    let invalid = compile_module(
        Path::new("list.tsx"),
        r#"
      import { VirtualList as List } from "@impasto/solid-ui";
      export const root = <List contentProps={{ style: { flexDirection: "row" } }} />;
    "#,
    );
    assert!(
        matches!(invalid, Err(error) if error.contains("flexDirection") && error.contains("list-content"))
    );
}

#[test]
fn emits_a_static_style_with_a_separate_opacity_binding() {
    let compiled = compile_module(
        Path::new("view.tsx"),
        r#"
      import { View } from "@impasto/solid-renderer/host";
      export const make = () => <View style={{ padding: 16, opacity: opacity() }} />;
    "#,
    )
    .unwrap();
    assert!(compiled.code.contains("defineStyle(\"1:"));
    assert!(
        compiled.code.contains("__impasto_jsx_styles[0], next)"),
        "{}",
        compiled.code
    );
    assert!(!compiled.code.contains("opacity: opacity()"));
}
