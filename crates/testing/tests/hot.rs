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
