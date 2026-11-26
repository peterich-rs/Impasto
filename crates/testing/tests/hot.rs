use std::{fs, path::Path};

use engine::{
    js::JsSession,
    session::{FrameTime, Metrics, ModuleConfig, Session},
    text::MonoShaper,
};

#[test]
fn edits_a_component_in_the_running_session() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    let child = root.join("Label.tsx");
    fs::write(
        &entry,
        r#"
        import { boot, Text } from "@impasto/solid-renderer";
        import { Label, next, read } from "./Label.tsx";
        import { View } from "@impasto/solid-renderer/host";
        next();
        boot(() => <View><Text>{read()}</Text><Label /></View>);
    "#,
    )
    .unwrap();
    let mut watch = bundler::WatchLinker::default();
    let mut bundle = |version: &str| {
        fs::write(
            &child,
            format!(
                r#"
            import {{ Text }} from "@impasto/solid-renderer";
            import {{ createSignal as signal }} from "solid-js";
            const [read, write] = signal(0);
            export {{ read }};
            export function next() {{ write(read() + 1); }}
            export const Label = () => <Text>{version} {{read()}}</Text>;
        "#
            ),
        )
        .unwrap();
        watch.set_changes(Some(vec![child.canonicalize().unwrap()]));
        watch
            .bundle(bundler::BundleOptions {
                root: root.to_path_buf(),
                entry: entry.clone(),
                framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
                hot: true,
                ..Default::default()
            })
            .unwrap()
            .code
    };
    let before = bundle("before");
    let after = bundle("after");
    let patch = bundler::delta(&before, &after).unwrap();
    assert!(patch.len() < after.len() / 2);
    let mut session = JsSession::open_with(
        &before,
        "app.js",
        MonoShaper::default(),
        Box::new(|_, _| {}),
        ModuleConfig::disconnected(),
    )
    .unwrap();
    session.metrics(&Metrics::new(360., 640., 1.));
    session.set_semantics_enabled(true);
    session.frame(FrameTime::at(0.));
    let semantics = session.semantics().unwrap();
    let original = semantics
        .nodes
        .iter()
        .find(|node| node.label == "1")
        .unwrap()
        .id;
    assert!(semantics.nodes.iter().any(|node| node.label == "before 1"));

    assert_eq!(session.apply_source_patch(&patch), Ok(true));
    assert!(session.apply_source_patch(&patch).is_err());
    session.frame(FrameTime::at(16.));
    assert!(session.take_faults().is_empty());
    let semantics = session.semantics().unwrap();
    assert!(
        semantics
            .nodes
            .iter()
            .any(|node| node.label == "1" && node.id == original),
        "{original:?} {:?}",
        semantics.nodes
    );
    assert!(semantics.nodes.iter().any(|node| node.label == "after 1"));
    assert!(!semantics.nodes.iter().any(|node| node.label == "before 1"));
}

#[test]
fn folded_source_edits_refresh_the_component_and_retain_inspector_locations() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    let palette = root.join("palette.ts");
    fs::write(
        &entry,
        r#"
import { boot } from "@impasto/solid-renderer";
import { View } from "@impasto/solid-ui";
import { theme } from "./palette";
export function Card() {
  return <View accessibilityLabel="Token card" style={{ width: theme.size.card }} />;
}
boot(() => <Card />);
"#,
    )
    .unwrap();
    let write = |width| {
        fs::write(
            &palette,
            format!("export const theme = {{ size: {{ card: {width} }} }} as const;"),
        )
        .unwrap()
    };
    write(20);
    let options = || bundler::BundleOptions {
        root: root.to_owned(),
        entry: entry.clone(),
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        hot: true,
        ..Default::default()
    };
    let mut watch = bundler::WatchLinker::default();
    let before = watch.bundle(options()).unwrap();
    let mut session = JsSession::open_with(
        &before.code,
        "app.js",
        MonoShaper::default(),
        Box::new(|_, _| {}),
        ModuleConfig::disconnected(),
    )
    .unwrap();
    session.metrics(&Metrics::new(360., 640., 1.));
    session.frame(FrameTime::at(0.));
    let initial = session.inspector_snapshot().unwrap();
    assert!(
        initial.contains("main.tsx") && initial.contains("Card") && initial.contains("\"w\":20.00"),
        "{initial}"
    );
    write(70);
    watch.set_changes(Some(vec![palette.canonicalize().unwrap()]));
    let after = watch.bundle(options()).unwrap();
    if let Some(patch) = bundler::delta(&before.code, &after.code) {
        assert_eq!(session.apply_source_patch(&patch), Ok(true));
    } else {
        session = JsSession::open_with(
            &after.code,
            "app.js",
            MonoShaper::default(),
            Box::new(|_, _| {}),
            ModuleConfig::disconnected(),
        )
        .unwrap();
        session.metrics(&Metrics::new(360., 640., 1.));
    }
    session.frame(FrameTime::at(16.));
    let changed = session.inspector_snapshot().unwrap();
    assert!(changed.contains("\"w\":70.00"), "{changed}");
    assert!(session.take_faults().is_empty());
}

#[test]
fn an_exported_default_arrow_tracks_its_helpers_and_symbolicates_each_revision() {
    use std::{cell::RefCell, rc::Rc};

    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    let child = root.join("Card.tsx");
    fs::write(
        &entry,
        r#"
        import { boot } from "@impasto/solid-renderer";
        import { Column } from "@impasto/solid-ui";
        import Card, { Stable } from "./Card.tsx";
        boot(() => <Column><Card /><Stable /></Column>);
    "#,
    )
    .unwrap();
    let mut watch = bundler::WatchLinker::default();
    let mut bundle = |value: &str| {
        fs::write(
            &child,
            format!(
                r#"import {{ Text }} from "@impasto/solid-ui";
const renderValue = () => "{value}";
const label = () => renderValue();
export const Stable = () => <Text>stable</Text>;
export default () => {{
  console.trace("factory trace");
  return <Text>{{label()}}</Text>;
}};
"#
            ),
        )
        .unwrap();
        watch.set_changes(Some(vec![child.canonicalize().unwrap()]));
        watch
            .bundle(bundler::BundleOptions {
                root: root.to_path_buf(),
                entry: entry.clone(),
                framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
                hot: true,
                ..Default::default()
            })
            .unwrap()
            .code
    };
    let before = bundle("before");
    let after = bundle("after");
    let patch = bundler::delta(&before, &after).unwrap();
    let logs = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&logs);
    let mut session = JsSession::open_with(
        &before,
        "app.js",
        MonoShaper::default(),
        Box::new(move |_, line| sink.borrow_mut().push(line.to_owned())),
        ModuleConfig::disconnected(),
    )
    .unwrap();
    session.metrics(&Metrics::new(360., 640., 1.));
    session.set_semantics_enabled(true);
    session.frame(FrameTime::at(0.));
    let stable = session
        .semantics()
        .unwrap()
        .nodes
        .into_iter()
        .find(|node| node.label == "stable")
        .unwrap()
        .id;
    assert_eq!(session.apply_source_patch(&patch), Ok(true));
    session.frame(FrameTime::at(16.));
    let nodes = session.semantics().unwrap().nodes;
    assert!(nodes.iter().any(|node| node.label == "after"));
    assert!(
        nodes
            .iter()
            .any(|node| node.label == "stable" && node.id == stable)
    );
    let logs = logs.borrow();
    let traces = logs
        .iter()
        .filter(|line| line.contains("factory trace"))
        .collect::<Vec<_>>();
    assert_eq!(traces.len(), 2);
    assert!(
        traces.iter().all(|line| line.contains("Card.tsx:6:11")),
        "{traces:?}"
    );
}
