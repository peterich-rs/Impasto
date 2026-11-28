use std::{fs, path::Path};

use bundler::DeltaOutcome;
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
    let patch = patch(&before, &after);
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
    if let DeltaOutcome::Patch { code: patch, .. } = bundler::delta(&before.code, &after.code) {
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
    let patch = patch(&before, &after);
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

fn patch(before: &str, after: &str) -> String {
    match bundler::delta(before, after) {
        DeltaOutcome::Patch { code, .. } => code,
        DeltaOutcome::FullReload { reason } => panic!("expected an HMR patch: {reason}"),
    }
}

#[test]
fn replays_pure_imports_and_adds_and_removes_modules_without_restarting() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    let card = root.join("Card.tsx");
    let helper = root.join("helper.ts");
    let extra = root.join("extra.ts");
    fs::write(
        &entry,
        r#"
        import { boot, Text } from "@impasto/solid-renderer";
        import { View } from "@impasto/solid-renderer/host";
        import { Card } from "./Card";
        boot(() => <View><Text>stable</Text><Card /></View>);
    "#,
    )
    .unwrap();
    let write_card = |with_extra| {
        let source = if with_extra {
            r#"
            import { Text } from "@impasto/solid-renderer";
            import { label } from "./helper";
            import { extra } from "./extra";
            export const Card = () => <Text>{label() + extra}</Text>;
            "#
        } else {
            r#"
            import { Text } from "@impasto/solid-renderer";
            import { label } from "./helper";
            export const Card = () => <Text>{label()}</Text>;
            "#
        };
        fs::write(&card, source).unwrap();
    };
    let options = || bundler::BundleOptions {
        root: root.to_path_buf(),
        entry: entry.clone(),
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        hot: true,
        ..Default::default()
    };
    write_card(false);
    fs::write(&helper, "export function label() { return 'one'; }").unwrap();
    let mut linker = bundler::WatchLinker::default();
    let mut prior = linker.bundle(options()).unwrap().code;
    let mut session = JsSession::open_with(
        &prior,
        "app.js",
        MonoShaper::default(),
        Box::new(|_, _| {}),
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
    fs::write(&helper, "export function label() { return 'two'; }").unwrap();
    for (iteration, expected) in ["two", "two!", "two"].into_iter().enumerate() {
        let changes = match iteration {
            0 => vec![helper.clone()],
            1 => {
                fs::write(&extra, "export const extra = '!';").unwrap();
                write_card(true);
                vec![card.clone(), extra.clone()]
            }
            _ => {
                write_card(false);
                fs::remove_file(&extra).unwrap();
                vec![card.clone(), extra.clone()]
            }
        };
        linker.set_changes(Some(changes));
        let next = linker.bundle(options()).unwrap().code;
        assert_eq!(session.apply_source_patch(&patch(&prior, &next)), Ok(true));
        session.frame(FrameTime::at((iteration + 1) as f64 * 16.));
        let nodes = session.semantics().unwrap().nodes;
        assert!(nodes.iter().any(|node| node.label == expected), "{nodes:?}");
        assert!(
            nodes
                .iter()
                .any(|node| node.label == "stable" && node.id == stable)
        );
        assert!(session.take_faults().is_empty());
        prior = next;
    }
}

#[test]
fn locale_and_theme_inputs_relink_their_consumers_in_a_live_session() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let entry = root.join("main.tsx");
    let card = root.join("Card.tsx");
    let locale = root.join("locales/en.json");
    let theme = root.join("src/theme.ts");
    fs::create_dir(root.join("locales")).unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        &entry,
        r#"
        import { boot, Text } from "@impasto/solid-renderer";
        import { View } from "@impasto/solid-renderer/host";
        import { Card } from "./Card";
        boot(() => <View><Text>stable</Text><Card /></View>);
    "#,
    )
    .unwrap();
    fs::write(
        &card,
        r#"
        import { t } from "@impasto/i18n";
        import { Text } from "@impasto/solid-renderer";
        import { theme } from "./src/theme";
        export const Card = () => <Text>{t("greeting")} {theme.label}</Text>;
    "#,
    )
    .unwrap();
    fs::write(&locale, r#"{"greeting":"Hello"}"#).unwrap();
    fs::write(&theme, "export const theme = { label: 'light' };").unwrap();
    let options = || bundler::BundleOptions {
        root: root.to_owned(),
        entry: entry.clone(),
        framework: Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        hot: true,
        ..Default::default()
    };
    let mut linker = bundler::WatchLinker::default();
    let mut source = linker.bundle(options()).unwrap().code;
    let mut session = JsSession::open_with(
        &source,
        "app.js",
        MonoShaper::default(),
        Box::new(|_, _| {}),
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
    for (path, contents, expected, time) in [
        (&locale, r#"{"greeting":"Welcome"}"#, "Welcome light", 16.),
        (
            &theme,
            "export const theme = { label: 'dark' };",
            "Welcome dark",
            32.,
        ),
    ] {
        fs::write(path, contents).unwrap();
        linker.set_changes(Some(vec![path.clone()]));
        let after = linker.bundle(options()).unwrap().code;
        assert_eq!(
            session.apply_source_patch(&patch(&source, &after)),
            Ok(true)
        );
        session.frame(FrameTime::at(time));
        let nodes = session.semantics().unwrap().nodes;
        assert!(nodes.iter().any(|node| node.label == expected), "{nodes:?}");
        assert!(
            nodes
                .iter()
                .any(|node| node.label == "stable" && node.id == stable)
        );
        assert!(session.take_faults().is_empty());
        source = after;
    }
}
