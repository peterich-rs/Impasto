//! Owned UI mutations and their commit on the render thread.

use crate::motion::ScrollAxis;
use crate::scene::{ContentMode, TextSpan};
use crate::style::codec::{self, Value};
use crate::tree::{DragClaim, NodeId, Tree};

use super::Host;
use super::nodes::dispatch_gestures;

pub(super) struct Applied {
    pub id: Option<NodeId>,
    pub source: Option<crate::style::StyleSourceId>,
    pub gestures: Vec<crate::tree::Gesture>,
    pub retired: Vec<NodeId>,
}

pub(super) enum Create {
    View,
    Stack,
    Scroll(ScrollAxis),
    Text {
        text: String,
    },
    Image {
        source: String,
        width: f32,
        height: f32,
        required: bool,
    },
}

pub(super) enum Mutation {
    StyleEnvironment(Value),
    Create(Create),
    Insert {
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    },
    Remove {
        parent: NodeId,
        child: NodeId,
    },
    Detach {
        parent: NodeId,
        child: NodeId,
    },
    Discard(NodeId),
    SetText {
        id: NodeId,
        text: String,
    },
    SetImage {
        id: NodeId,
        source: String,
        intrinsic: Option<(f32, f32)>,
    },
    SetStyleSurface {
        id: NodeId,
        surface: crate::style::StyleSurface,
    },
    TextRuns {
        id: NodeId,
        spans: Vec<TextSpan>,
    },
    Focusable {
        id: NodeId,
        enabled: bool,
    },
    DragClaim {
        id: NodeId,
        claim: DragClaim,
    },
    TextLimit {
        id: NodeId,
        limit: Option<u32>,
    },
    ScrollOffset {
        id: NodeId,
        x: f32,
        y: f32,
    },
    ScrollAxis {
        id: NodeId,
        axis: ScrollAxis,
    },
    ScrollChain {
        id: NodeId,
        enabled: bool,
    },
    Reparent {
        from: NodeId,
        to: NodeId,
    },
    ClassSnapshot {
        id: NodeId,
        text: String,
        list: bool,
    },
    PointerTarget(NodeId),
    ClearStyle {
        id: NodeId,
        keys: Option<Vec<crate::style::Key>>,
    },
    SourceClass {
        source: crate::style::StyleSourceId,
        text: String,
    },
    StyleSnapshot {
        id: NodeId,
        origin: crate::style::StyleOrigin,
        replace: bool,
        style: crate::style::Style,
    },
    SourceCreate {
        id: NodeId,
        origin: crate::style::StyleOrigin,
    },
    SourceRemove(crate::style::StyleSourceId),
    SourceSnapshot {
        source: crate::style::StyleSourceId,
        style: crate::style::Style,
    },
    SetCanvas {
        id: NodeId,
        list: Option<crate::scene::CanvasList>,
    },
}

pub(super) fn class_snapshot(
    id: NodeId,
    list: bool,
    value: Value,
) -> Result<Mutation, crate::Error> {
    let text = match value {
        Value::Null | Value::Undefined => String::new(),
        Value::String(text) if !list => text,
        Value::Object(fields) if list => {
            let mut names = Vec::new();
            for (key, value) in fields {
                match value {
                    Value::Bool(true) => names.push(key),
                    Value::Bool(false) | Value::Undefined => {}
                    _ => return Err(crate::Error::BadNumber),
                }
            }
            names.join(" ")
        }
        _ => return Err(crate::Error::BadNumber),
    };
    Ok(Mutation::ClassSnapshot { id, text, list })
}

pub(super) fn clear_style(id: NodeId, value: Value) -> Result<Mutation, crate::Error> {
    let keys = match value {
        Value::Null | Value::Undefined => None,
        Value::Array(items) => {
            let mut keys = Vec::new();
            for item in items {
                let Value::String(name) = item else {
                    return Err(crate::Error::BadNumber);
                };
                keys.extend(
                    codec::declarations(&name, &Value::Null)?
                        .into_iter()
                        .map(|item| item.key()),
                );
            }
            Some(keys)
        }
        _ => return Err(crate::Error::BadNumber),
    };
    Ok(Mutation::ClearStyle { id, keys })
}

pub(super) fn apply(tree: &mut Tree, mutation: Mutation) -> Result<Applied, crate::Error> {
    let mut applied = Applied {
        id: None,
        source: None,
        gestures: Vec::new(),
        retired: Vec::new(),
    };
    match mutation {
        Mutation::StyleEnvironment(value) => super::environment::apply(tree, &value)?,
        Mutation::ClassSnapshot { id, text, list } => {
            if list {
                tree.set_class_list(id, &text)?
            } else {
                tree.set_class(id, &text)?
            }
        }
        Mutation::PointerTarget(id) => tree.set_pointer_target(id)?,
        Mutation::ClearStyle { id, keys } => tree.clear_style(id, keys.as_deref())?,
        Mutation::SourceClass { source, text } => tree.replace_style_source_class(source, &text)?,
        Mutation::StyleSnapshot {
            id,
            origin,
            replace,
            style,
        } => {
            tree.commit_style_snapshot(id, origin, style, replace)?;
        }
        Mutation::SourceCreate { id, origin } => {
            applied.source = Some(tree.create_style_source(id, origin)?)
        }
        Mutation::SourceRemove(source) => tree.remove_style_source(source)?,
        Mutation::SourceSnapshot { source, style } => tree.replace_style_source(source, style)?,
        Mutation::Create(create) => {
            applied.id = Some(create_node(tree, create)?);
        }
        Mutation::Insert {
            parent,
            child,
            before,
        } => tree.insert(parent, child, before)?,
        Mutation::Remove { parent, child } => {
            applied.gestures = tree.remove(parent, child)?;
        }
        Mutation::Detach { parent, child } => {
            applied.gestures = tree.detach(parent, child)?;
        }
        Mutation::Discard(id) => {
            let discarded = tree.discard(id)?;
            applied.gestures = discarded.gestures;
            applied.retired = discarded.ids;
        }
        Mutation::SetText { id, text } => tree.set_text(id, text)?,
        Mutation::SetImage {
            id,
            source,
            intrinsic,
        } => {
            tree.set_image(id, &source, intrinsic)?;
        }
        Mutation::SetStyleSurface { id, surface } => tree.set_style_surface(id, surface)?,
        Mutation::TextRuns { id, spans } => tree.set_text_spans(id, spans)?,
        Mutation::Focusable { id, enabled } => {
            applied.gestures = tree.set_focusable(id, enabled)?
        }
        Mutation::DragClaim { id, claim } => tree.set_drag_claim(id, claim)?,
        Mutation::TextLimit { id, limit } => tree.set_text_limit(id, limit)?,
        Mutation::ScrollOffset { id, x, y } => tree.set_scroll_offset(id, x, y)?,
        Mutation::ScrollAxis { id, axis } => tree.set_scroll_axis(id, axis)?,
        Mutation::ScrollChain { id, enabled } => tree.set_scroll_chain(id, enabled)?,
        Mutation::Reparent { from, to } => tree.reparent_children(from, to)?,
        Mutation::SetCanvas { id, list } => {
            let clearing = list.is_none();
            match tree.set_canvas(id, list) {
                Err(crate::Error::UnknownNode) if clearing => {}
                other => other?,
            }
        }
    }
    applied.gestures.extend(tree.take_style_gestures());
    Ok(applied)
}

fn create_node(tree: &mut Tree, create: Create) -> Result<NodeId, crate::Error> {
    match create {
        Create::View => tree.view(),
        Create::Stack => tree.stack(),
        Create::Scroll(axis) => tree.scroll_with(axis),
        Create::Text { text } => tree.text(text, 16.0, [24, 24, 27, 255]),
        Create::Image {
            source,
            width,
            height,
            required,
        } => {
            let image = tree.image(source, width, height)?;
            if required {
                tree.set_image_mode(image, ContentMode::Fill, true)?;
            }
            Ok(image)
        }
    }
}

fn apply_now(host: &Host, mutation: Mutation) -> Result<Applied, crate::Error> {
    let inner = host.inner();
    let trace = inner.marks.trace_enabled.get();
    let started = trace.then(std::time::Instant::now);
    // Read the authoritative node before invoking a setter. Equal writes do
    // not cross into Tree::set_*. No JS property cache or second tree is kept.
    let unchanged = {
        let tree = inner.tree.borrow();
        match &mutation {
            Mutation::SetText { id, text } => tree.plain_text(*id)? == Some(text.as_str()),
            _ => false,
        }
    };
    if unchanged {
        if let Some(started) = started {
            super::frame::bump(&inner.marks.mutation_calls);
            super::frame::bump(&inner.marks.mutation_equal);
            inner
                .marks
                .mutation_time
                .set(inner.marks.mutation_time.get() + started.elapsed());
        }
        return Ok(Applied {
            id: None,
            source: None,
            gestures: Vec::new(),
            retired: Vec::new(),
        });
    }
    let styled = matches!(
        mutation,
        Mutation::TextRuns { .. }
            | Mutation::Focusable { .. }
            | Mutation::DragClaim { .. }
            | Mutation::TextLimit { .. }
            | Mutation::ScrollOffset { .. }
            | Mutation::ScrollChain { .. }
            | Mutation::ScrollAxis { .. }
            | Mutation::SetText { .. }
            | Mutation::SetImage { .. }
            | Mutation::SetStyleSurface { .. }
    );
    let text_write = trace && matches!(mutation, Mutation::SetText { .. });
    let applied = {
        let mut tree = inner.tree.borrow_mut();
        apply(&mut tree, mutation)?
    };
    if let Some(started) = started {
        super::frame::bump(&inner.marks.mutation_calls);
        if text_write {
            super::frame::bump(&inner.marks.text_writes);
        }
        inner
            .marks
            .mutation_time
            .set(inner.marks.mutation_time.get() + started.elapsed());
    }
    if styled {
        super::frame::bump(&inner.marks.sets);
    }
    if let Err(message) = dispatch_gestures(host, applied.gestures.clone()) {
        (inner.log)(3, &message);
    }
    forget(host, &applied.retired);
    Ok(applied)
}

fn forget(host: &Host, ids: &[NodeId]) {
    if ids.is_empty() {
        return;
    }
    let raw: Vec<u64> = ids.iter().copied().map(NodeId::to_raw).collect();
    host.forget_ids(&raw);
}

pub(super) fn mutate(host: &Host, mutation: Mutation) -> Result<u64, crate::Error> {
    let applied = apply_now(host, mutation)?;
    Ok(applied
        .id
        .map(NodeId::to_raw)
        .or_else(|| applied.source.map(crate::style::StyleSourceId::to_raw))
        .unwrap_or(0))
}

/// Commits pending style and layout work before reading a rectangle.
pub(super) fn flush_for_read(host: &Host) -> bool {
    let inner = host.inner();
    let _trace = crate::session::trace::Section::begin(c"Impasto synchronous geometry");
    let notices = {
        let mut tree = inner.tree.borrow_mut();
        if tree.flush_styles().is_err() {
            return false;
        }
        tree.take_style_gestures()
    };
    if dispatch_gestures(host, notices).is_err() {
        return false;
    }
    let mut tree = inner.tree.borrow_mut();
    !tree.needs_layout() || tree.layout().is_ok()
}
