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
        compiled.code.contains("bindScalarStyle"),
        "{}",
        compiled.code
    );
    assert!(!compiled.code.contains("opacity: opacity()"));
}

#[test]
fn lowers_images_refs_and_direct_lists() -> Result<(), String> {
    let compiled = compile_module(
        Path::new("view.tsx"),
        r#"
        import { For as Each } from "solid-js";
        import { View, Image, Text } from "@impasto/solid-ui";
        export const card = () => <View ref={parent}>
            <Image source="asset:icon.png" intrinsicSize={{ width: 24, height: 24 }} alt="Icon" ref={child} />
            <View><Each each={items()}>{item => <Text>{item}</Text>}</Each></View>
        </View>;
    "#,
    )?;
    assert!(compiled.code.contains("bindTemplate"), "{}", compiled.code);
    assert!(
        compiled.code.contains("asset:icon.png"),
        "{}",
        compiled.code
    );
    assert!(
        !compiled.code.contains("bindImageSource"),
        "{}",
        compiled.code
    );
    assert!(
        compiled.code.contains("insertKeyedList"),
        "{}",
        compiled.code
    );
    assert!(
        compiled
            .code
            .find("=> child")
            .zip(compiled.code.find("=> parent"))
            .is_some_and(|(child, parent)| child < parent),
        "{}",
        compiled.code
    );
    Ok(())
}

#[test]
fn pools_read_only_style_bindings_and_keeps_mutated_objects_live() -> Result<(), String> {
    let compiled = compile_module(
        Path::new("styles.tsx"),
        r##"
        import { Row, Text } from "@impasto/solid-ui";
        const palette = { ink: "#224466" };
        const spacing = 8;
        const card = { width: 30 };
        const mutable = { width: 20 };
        mutable.width = 40;
        export const Card = () => <Row style={card}>
          <Text style={{ color: palette.ink, fontSize: spacing }}>Price: {price()}</Text>
          <Row style={mutable} />
        </Row>;
    "##,
    )?;
    assert!(compiled.code.contains("void card"), "{}", compiled.code);
    assert!(compiled.code.contains("void palette"), "{}", compiled.code);
    assert!(compiled.code.contains("void spacing"), "{}", compiled.code);
    assert!(!compiled.code.contains("void mutable"), "{}", compiled.code);
    assert!(compiled.code.contains("bindTextValue"), "{}", compiled.code);
    assert!(!compiled.code.contains(".memo") && !compiled.code.contains("createComponent"));
    Ok(())
}

#[test]
fn reports_only_synchronous_component_reads_and_calls_after_await() -> Result<(), String> {
    let compiled = compile_module(
        Path::new("reactivity.tsx"),
        r#"
        import { Text } from "@impasto/solid-ui";
        import { createSignal as signal, onCleanup as cleanup } from "solid-js";
        const [read] = signal(0);
        export function Label(props) {
          const label = props.label ?? "default";
          const value = read();
          const callback = (props) => props.label;
          return <Text>{props.label}{read()}{label + value}</Text>;
        }
        export async function watch(first) {
          if (first) { await start(); }
          else { cleanup(() => {}); }
          cleanup(() => {});
          await start();
          cleanup(() => {});
        }
    "#,
    )?;
    assert_eq!(compiled.warnings.len(), 3, "{:?}", compiled.warnings);
    assert!(
        compiled
            .warnings
            .iter()
            .any(|warning| warning.contains("prop only once"))
    );
    assert!(
        compiled
            .warnings
            .iter()
            .any(|warning| warning.contains("accessor only once"))
    );
    assert!(
        compiled
            .warnings
            .iter()
            .any(|warning| warning.contains("unavailable after await"))
    );
    Ok(())
}
