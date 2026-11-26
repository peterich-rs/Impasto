use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use engine::{
    input::PointerEvent,
    js::JsSession,
    semantics::{SemanticsAction, SemanticsNode, SemanticsRole},
    session::{FrameTime, Metrics, ModuleConfig, ModuleRegistry, Session},
    text::MonoShaper,
};

#[test]
fn gallery_patch_preserves_the_tab_scroll_and_focused_field() -> Result<()> {
    let framework = crate::toolchain::framework()?;
    let workspace = tempfile::tempdir()?;
    let root = workspace.path();
    let source = framework.join(crate::GALLERY_DIR).join("src");
    copy_sources(&source, &root.join("src"))?;

    let modules = super::modules(root)?;
    let externals: std::collections::BTreeSet<_> = modules
        .iter()
        .map(|module| module.specifier.to_owned())
        .collect();
    let options = || bundler::BundleOptions {
        framework: framework.clone(),
        root: root.to_owned(),
        entry: root.join("src/quotes.tsx"),
        externals: externals.clone(),
        hot: true,
        ..Default::default()
    };
    let mut linker = bundler::WatchLinker::default();
    let before = linker.bundle(options())?.code;
    let mut session = JsSession::open_with(
        &before,
        "app.js",
        MonoShaper::default(),
        Box::new(|level, message| {
            if level >= 3 {
                eprintln!("{message}");
            }
        }),
        ModuleConfig {
            registry: ModuleRegistry::new(modules).into(),
            ..ModuleConfig::disconnected()
        },
    )?;

    let mut clock = 0.0;
    let session = &mut session;
    session.set_semantics_enabled(true);
    session.metrics(&Metrics::new(430.0, 440.0, 1.0));
    advance(session, &mut clock)?;
    let scroll = node(session, |node| {
        node.role == SemanticsRole::Scroll
            && node
                .scroll_content
                .is_some_and(|(_, height)| height > node.rect.h + 1.0)
    })
    .context("the watchlist needs a scrollable viewport")?;
    let x = scroll.rect.x + scroll.rect.w * 0.5;
    let y = scroll.rect.y + scroll.rect.h * 0.5;
    session.pointer(PointerEvent::wheel(x, y, clock, 0.0, 212.0));
    advance(session, &mut clock)?;
    let offset = node(session, |node| node.id == scroll.id)?.scroll;
    ensure!(
        offset.is_some_and(|(_, y)| y > 0.0),
        "watchlist did not scroll: {offset:?}"
    );

    activate(session, &mut clock, "搜索")?;
    let field = node(session, |node| node.role == SemanticsRole::SearchField)
        .context("opening search did not mount its field")?;
    session.semantics_action(SemanticsAction::Focus(field.id));
    session.semantics_action(SemanticsAction::SetText {
        id: field.id,
        text: "700".into(),
    });
    advance(session, &mut clock)?;
    let edited = node(session, |node| node.id == field.id)?;
    ensure!(
        edited.value.as_deref() == Some("700"),
        "input did not reach the field: {edited:?}"
    );

    let file = root.join("src/pages/search.tsx").canonicalize()?;
    let previous = fs::read_to_string(&file)?;
    let next = previous.replace(
        r#"<Label size={15} weight={600}>{code(props.instrument.symbol)}</Label>"#,
        r#"<Label size={16} weight={600}>{code(props.instrument.symbol)} 更新</Label>"#,
    );
    ensure!(next != previous, "missing Gallery search result edit point");
    fs::write(&file, next)?;
    linker.set_changes(Some(vec![file]));
    let after = linker.bundle(options())?.code;
    let patch = bundler::delta(&before, &after).context("Gallery edit generated a full reload")?;
    ensure!(
        session
            .apply_source_patch(&patch)
            .map_err(anyhow::Error::msg)?,
        "patch was ignored"
    );
    advance(session, &mut clock)?;
    node(session, |node| node.label == "00700 更新")?;
    let retained = node(session, |node| node.id == field.id)?;
    assert_eq!(retained.value, edited.value);
    assert_eq!(retained.caret, edited.caret);

    activate(session, &mut clock, "返回")?;
    let restored = node(session, |node| node.id == scroll.id)
        .context("returning to the watchlist did not restore its scroll node")?;
    assert_eq!(restored.scroll, offset);
    Ok(())
}

fn copy_sources(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let output = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_sources(&entry.path(), &output)?;
        } else {
            fs::copy(entry.path(), output)?;
        }
    }
    Ok(())
}

fn advance(session: &mut JsSession, time: &mut f64) -> Result<()> {
    for _ in 0..32 {
        *time += 16.0;
        session.frame(FrameTime::at(*time));
        ensure!(session.take_script_error().is_none(), "script failed");
        let faults = session.take_faults();
        ensure!(faults.is_empty(), "{faults:?}");
    }
    Ok(())
}

fn node(session: &mut JsSession, pick: impl Fn(&SemanticsNode) -> bool) -> Result<SemanticsNode> {
    let snapshot = session
        .semantics()
        .context("Gallery semantics unavailable")?;
    snapshot
        .nodes
        .iter()
        .find(|node| pick(node))
        .cloned()
        .with_context(|| {
            let visible = snapshot
                .nodes
                .iter()
                .filter(|node| !node.label.is_empty())
                .map(|node| (&node.label, node.role))
                .collect::<Vec<_>>();
            format!("Gallery semantics did not contain the requested node: {visible:?}")
        })
}

fn activate(session: &mut JsSession, time: &mut f64, label: &str) -> Result<()> {
    let button = node(session, |node| {
        node.role == SemanticsRole::Button && node.label == label
    })
    .with_context(|| format!("missing Gallery button {label}"))?;
    session.semantics_action(SemanticsAction::Activate(button.id));
    advance(session, time)
}
