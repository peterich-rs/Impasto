//! Tree writes. One record, applied immediately or from the frame queue.
//!
//! Adding a style field is a [`StyleChange`] arm plus a line in `abi.yml`.
//! The C function table does not grow.

use std::ffi::{c_char, c_void};

use protocol::{TextAlign, TextSpan};
use uicore::{
    Alignment, AxisSize, Edges, GridLine, Inset, Justify, NodeId, RowCrossAlign, TrackSize,
};

use super::ffi::qjs_forget_ids;
use super::nodes::{dispatch_gestures, parse_tracks};
use super::{Inner, inner, utf8_of};

use crate::abi::{CreateKind, MutationOp, StyleField};

/// What a successful write produced. Gestures are dispatched before JS continues.
pub(super) struct Applied {
    pub id: Option<NodeId>,
    pub gestures: Vec<uicore::Gesture>,
    pub retired: Vec<NodeId>,
}

pub(super) enum Create {
    Column {
        gap: f32,
    },
    Row {
        gap: f32,
    },
    Stack,
    Padding {
        edges: f32,
    },
    Sized {
        width: f32,
        height: f32,
    },
    Scroll {
        axis: i32,
    },
    Align(Alignment),
    Grid {
        columns: Vec<TrackSize>,
        rows: Vec<TrackSize>,
        gap: f32,
    },
    Pointer,
    Decorated {
        color: [u8; 4],
        radius: f32,
    },
    Clip {
        radius: f32,
    },
    Opacity {
        alpha: u8,
    },
    Transform,
    Image {
        source: String,
        width: f32,
        height: f32,
    },
    Text {
        utf8: String,
        font_size: f32,
        color: [u8; 4],
    },
}

pub(super) enum StyleChange {
    Width(f32),
    Height(f32),
    Size {
        width: f32,
        height: f32,
    },
    Padding(f32),
    PaddingEdges(Edges),
    PaddingEdge {
        edge: u8,
        value: f32,
    },
    Margin(Edges),
    Gap(f32),
    Grow(f32),
    Shrink(f32),
    Basis(f32),
    MinSize {
        width: f32,
        height: f32,
    },
    MaxSize {
        width: f32,
        height: f32,
    },
    AlignSelf(Alignment),
    ContainerAlign(Alignment),
    Justify(Justify),
    Wrap(bool),
    Inset(Inset),
    InsetEdge {
        edge: u8,
        value: Option<f32>,
    },
    Grid,
    GridLine {
        edge: u8,
        line: GridLine,
    },
    FontSize(f32),
    TextAlign(TextAlign),
    Weight(u16),
    MaxLines(Option<u32>),
    LineHeight(f32),
    Color([u8; 4]),
    Radius(f32),
    Stroke {
        width: f32,
        color: [u8; 4],
    },
    Alpha(u8),
    Blur(f32),
    Family(Option<String>),
    Ellipsis(Option<String>),
    SoftWrap(bool),
    Overflow(i32),
    Direction(bool),
    Reverse(bool),
    Position(bool),
    Aspect(Option<f32>),
    Cross(RowCrossAlign),
    FlexFill,
    ScrollOffset {
        x: f32,
        y: f32,
    },
    ScrollAxis(i32),
    ScrollChain(bool),
    ImageMode {
        mode: i32,
        required: bool,
    },
    ScaleXY {
        x: f32,
        y: f32,
    },
    Transform {
        x: f32,
        y: f32,
        rotate: f32,
        scale: f32,
    },
    TransformPart {
        component: u8,
        value: f32,
    },
    SizeAxis {
        axis: u8,
        size: AxisSize,
    },
    AxisSize {
        width: AxisSize,
        height: AxisSize,
    },
    MinAxis {
        width: AxisSize,
        height: AxisSize,
    },
    MaxAxis {
        width: AxisSize,
        height: AxisSize,
    },
    Focusable(bool),
    DragClaim(i32),
    TextLimit(Option<u32>),
    Panel,
    TextRuns(Vec<TextSpan>),
}

pub(super) enum Mutation {
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
        width: f32,
        height: f32,
    },
    EditText {
        id: NodeId,
        edit: uicore::Edit,
    },
    Style {
        id: NodeId,
        change: StyleChange,
    },
    Reparent {
        from: NodeId,
        to: NodeId,
    },
    Recognizers {
        id: NodeId,
        tap: i32,
        long_press: i32,
        drag: i32,
        window_ms: f64,
    },
}

/// Wire record shared with `glue.c`. Field order is the C layout.
#[repr(C)]
pub(in crate::linked) struct MutRec {
    pub op: u32,
    pub field: u32,
    pub id: u64,
    pub parent: u64,
    pub before: u64,
    pub number: f64,
    pub number2: f64,
    pub number3: f64,
    pub number4: f64,
    pub color: [u8; 4],
    pub _pad: u32,
    pub text: *const c_char,
    pub text_len: usize,
}

pub(super) struct OwnedRec {
    pub op: u32,
    pub field: u32,
    pub id: u64,
    pub parent: u64,
    pub before: u64,
    pub number: f64,
    pub number2: f64,
    pub number3: f64,
    pub number4: f64,
    pub color: [u8; 4],
    pub text: String,
}

impl OwnedRec {
    pub(super) fn from_queued(
        op: u32,
        id: u64,
        parent: u64,
        before: u64,
        field: u32,
        number: f64,
        number2: f64,
        color: [u8; 4],
        text: String,
    ) -> Self {
        Self {
            op,
            field,
            id,
            parent,
            before,
            number,
            number2,
            number3: 0.0,
            number4: 0.0,
            color,
            text,
        }
    }
}

pub(super) fn parse(rec: &OwnedRec) -> Result<Mutation, uicore::Error> {
    let id = NodeId::from_raw(rec.id);
    let op = rec.op;
    if op == MutationOp::Create as u32 {
        return Ok(Mutation::Create(parse_create(rec)?));
    }
    if op == MutationOp::Insert as u32 {
        return Ok(Mutation::Insert {
            parent: NodeId::from_raw(rec.parent),
            child: id,
            before: (rec.before != 0).then(|| NodeId::from_raw(rec.before)),
        });
    }
    if op == MutationOp::Remove as u32 {
        return Ok(Mutation::Remove {
            parent: NodeId::from_raw(rec.parent),
            child: id,
        });
    }
    if op == MutationOp::Detach as u32 {
        return Ok(Mutation::Detach {
            parent: NodeId::from_raw(rec.parent),
            child: id,
        });
    }
    if op == MutationOp::Discard as u32 {
        return Ok(Mutation::Discard(id));
    }
    if op == MutationOp::SetText as u32 {
        return Ok(Mutation::SetText {
            id,
            text: rec.text.clone(),
        });
    }
    if op == MutationOp::SetImage as u32 {
        return Ok(Mutation::SetImage {
            id,
            source: rec.text.clone(),
            width: rec.number as f32,
            height: rec.number2 as f32,
        });
    }
    if op == MutationOp::EditText as u32 {
        let edit = match rec.number as i32 {
            0 => uicore::Edit::Composing(None),
            1 => uicore::Edit::Insert(rec.text.clone()),
            2 => uicore::Edit::Backspace,
            3 => uicore::Edit::ForwardDelete,
            _ => return Err(uicore::Error::BadNumber),
        };
        return Ok(Mutation::EditText { id, edit });
    }
    if op == MutationOp::Reparent as u32 {
        return Ok(Mutation::Reparent {
            from: id,
            to: NodeId::from_raw(rec.parent),
        });
    }
    if op == MutationOp::SetRecognizers as u32 {
        return Ok(Mutation::Recognizers {
            id,
            tap: rec.number as i32,
            long_press: rec.number2 as i32,
            drag: rec.number3 as i32,
            window_ms: rec.number4,
        });
    }
    if op == MutationOp::SetSemantics as u32 {
        return Err(uicore::Error::BadNumber);
    }
    if op != MutationOp::SetStyle as u32 {
        return Err(uicore::Error::BadNumber);
    }
    Ok(Mutation::Style {
        id,
        change: parse_style(rec)?,
    })
}

fn parse_create(rec: &OwnedRec) -> Result<Create, uicore::Error> {
    let kind = rec.field;
    if kind == CreateKind::Column as u32 {
        return Ok(Create::Column {
            gap: rec.number as f32,
        });
    }
    if kind == CreateKind::Row as u32 {
        return Ok(Create::Row {
            gap: rec.number as f32,
        });
    }
    if kind == CreateKind::Stack as u32 {
        return Ok(Create::Stack);
    }
    if kind == CreateKind::Padding as u32 {
        return Ok(Create::Padding {
            edges: rec.number as f32,
        });
    }
    if kind == CreateKind::Sized as u32 {
        return Ok(Create::Sized {
            width: rec.number as f32,
            height: rec.number2 as f32,
        });
    }
    if kind == CreateKind::Scroll as u32 {
        return Ok(Create::Scroll {
            axis: rec.number as i32,
        });
    }
    if kind == CreateKind::Align as u32 {
        return Ok(Create::Align(alignment(rec.number as u32)?));
    }
    if kind == CreateKind::Grid as u32 {
        let (columns, rows) = split_tracks(&rec.text)?;
        return Ok(Create::Grid {
            columns: parse_tracks(columns)?,
            rows: parse_tracks(rows)?,
            gap: rec.number as f32,
        });
    }
    if kind == CreateKind::Pointer as u32 {
        return Ok(Create::Pointer);
    }
    if kind == CreateKind::Decorated as u32 {
        return Ok(Create::Decorated {
            color: rec.color,
            radius: rec.number as f32,
        });
    }
    if kind == CreateKind::Clip as u32 {
        return Ok(Create::Clip {
            radius: rec.number as f32,
        });
    }
    if kind == CreateKind::Opacity as u32 {
        return Ok(Create::Opacity {
            alpha: alpha_byte(rec.number)?,
        });
    }
    if kind == CreateKind::Transform as u32 {
        return Ok(Create::Transform);
    }
    if kind == CreateKind::Image as u32 {
        return Ok(Create::Image {
            source: rec.text.clone(),
            width: rec.number as f32,
            height: rec.number2 as f32,
        });
    }
    if kind == CreateKind::Text as u32 {
        return Ok(Create::Text {
            utf8: rec.text.clone(),
            font_size: rec.number as f32,
            color: rec.color,
        });
    }
    Err(uicore::Error::BadNumber)
}

fn parse_style(rec: &OwnedRec) -> Result<StyleChange, uicore::Error> {
    let field = rec.field;
    let value = rec.number as f32;
    if field == StyleField::Width as u32 {
        return Ok(StyleChange::Width(value));
    }
    if field == StyleField::Height as u32 {
        return Ok(StyleChange::Height(value));
    }
    if field == StyleField::Size as u32 {
        return Ok(StyleChange::Size {
            width: value,
            height: rec.number2 as f32,
        });
    }
    if field == StyleField::Padding as u32 {
        return Ok(StyleChange::Padding(value));
    }
    if field == StyleField::PaddingEdges as u32 {
        return Ok(StyleChange::PaddingEdges(edges_of(rec)));
    }
    if field == StyleField::PaddingEdge as u32 {
        return Ok(StyleChange::PaddingEdge {
            edge: rec.number as u8,
            value: rec.number2 as f32,
        });
    }
    if field == StyleField::Margin as u32 {
        return Ok(StyleChange::Margin(edges_of(rec)));
    }
    if field == StyleField::Gap as u32 {
        return Ok(StyleChange::Gap(value));
    }
    if field == StyleField::Grow as u32 {
        return Ok(StyleChange::Grow(value));
    }
    if field == StyleField::Shrink as u32 {
        return Ok(StyleChange::Shrink(value));
    }
    if field == StyleField::Basis as u32 {
        return Ok(StyleChange::Basis(value));
    }
    if field == StyleField::MinSize as u32 {
        return Ok(StyleChange::MinSize {
            width: value,
            height: rec.number2 as f32,
        });
    }
    if field == StyleField::MaxSize as u32 {
        return Ok(StyleChange::MaxSize {
            width: value,
            height: rec.number2 as f32,
        });
    }
    if field == StyleField::Align as u32 {
        return Ok(StyleChange::AlignSelf(alignment(rec.number as u32)?));
    }
    if field == StyleField::ContainerAlign as u32 {
        return Ok(StyleChange::ContainerAlign(alignment(rec.number as u32)?));
    }
    if field == StyleField::Justify as u32 {
        return Ok(StyleChange::Justify(justify(rec.number as u32)?));
    }
    if field == StyleField::Wrap as u32 {
        return Ok(StyleChange::Wrap(rec.number > 0.5));
    }
    if field == StyleField::Inset as u32 {
        return Ok(StyleChange::Inset(inset_of(rec)));
    }
    if field == StyleField::InsetEdge as u32 {
        return Ok(StyleChange::InsetEdge {
            edge: rec.number as u8,
            value: (rec.number3 == 0.0).then_some(rec.number2 as f32),
        });
    }
    if field == StyleField::Grid as u32 {
        return Ok(StyleChange::Grid);
    }
    if field == StyleField::GridLine as u32 {
        return Ok(StyleChange::GridLine {
            edge: rec.number as u8,
            line: grid_line(rec.number2 as i32, rec.number3 as i32)?,
        });
    }
    if field == StyleField::FontSize as u32 {
        return Ok(StyleChange::FontSize(value));
    }
    if field == StyleField::TextAlign as u32 {
        return Ok(StyleChange::TextAlign(text_align(rec.number as u32)));
    }
    if field == StyleField::Weight as u32 {
        return Ok(StyleChange::Weight(value.round().clamp(0.0, 1000.0) as u16));
    }
    if field == StyleField::MaxLines as u32 {
        return Ok(StyleChange::MaxLines(
            (rec.number > 0.0).then_some(rec.number.round() as u32),
        ));
    }
    if field == StyleField::LineHeight as u32 {
        return Ok(StyleChange::LineHeight(value));
    }
    if field == StyleField::Fill as u32 || field == StyleField::Color as u32 {
        return Ok(StyleChange::Color(rec.color));
    }
    if field == StyleField::Radius as u32 {
        return Ok(StyleChange::Radius(value));
    }
    if field == StyleField::Stroke as u32 {
        return Ok(StyleChange::Stroke {
            width: value,
            color: rec.color,
        });
    }
    if field == StyleField::Alpha as u32 {
        return Ok(StyleChange::Alpha(alpha_byte(rec.number)?));
    }
    if field == StyleField::Blur as u32 {
        return Ok(StyleChange::Blur(value));
    }
    if field == StyleField::Family as u32 {
        return Ok(StyleChange::Family(
            (!rec.text.is_empty()).then(|| rec.text.clone()),
        ));
    }
    if field == StyleField::Ellipsis as u32 {
        return Ok(StyleChange::Ellipsis(
            (!rec.text.is_empty()).then(|| rec.text.clone()),
        ));
    }
    if field == StyleField::SoftWrap as u32 {
        return Ok(StyleChange::SoftWrap(rec.number > 0.5));
    }
    if field == StyleField::Overflow as u32 {
        return Ok(StyleChange::Overflow(rec.number as i32));
    }
    if field == StyleField::Direction as u32 {
        return Ok(StyleChange::Direction(rec.number > 0.5));
    }
    if field == StyleField::Reverse as u32 {
        return Ok(StyleChange::Reverse(rec.number > 0.5));
    }
    if field == StyleField::Position as u32 {
        return Ok(StyleChange::Position(rec.number > 0.5));
    }
    if field == StyleField::Aspect as u32 {
        return Ok(StyleChange::Aspect(
            (rec.number > 0.5).then_some(rec.number2 as f32),
        ));
    }
    if field == StyleField::Cross as u32 {
        return Ok(StyleChange::Cross(cross(rec.number as u32)?));
    }
    if field == StyleField::FlexFill as u32 {
        return Ok(StyleChange::FlexFill);
    }
    if field == StyleField::ScrollOffset as u32 {
        return Ok(StyleChange::ScrollOffset {
            x: value,
            y: rec.number2 as f32,
        });
    }
    if field == StyleField::ScrollAxis as u32 {
        return Ok(StyleChange::ScrollAxis(rec.number as i32));
    }
    if field == StyleField::ScrollChain as u32 {
        return Ok(StyleChange::ScrollChain(rec.number > 0.5));
    }
    if field == StyleField::ImageMode as u32 {
        return Ok(StyleChange::ImageMode {
            mode: rec.number as i32,
            required: rec.number2 > 0.5,
        });
    }
    if field == StyleField::ScaleXY as u32 {
        return Ok(StyleChange::ScaleXY {
            x: value,
            y: rec.number2 as f32,
        });
    }
    if field == StyleField::Transform as u32 {
        return Ok(StyleChange::Transform {
            x: value,
            y: rec.number2 as f32,
            rotate: rec.number3 as f32,
            scale: rec.number4 as f32,
        });
    }
    if field == StyleField::TransformPart as u32 {
        return Ok(StyleChange::TransformPart {
            component: rec.number as u8,
            value: rec.number2 as f32,
        });
    }
    if field == StyleField::SizeAxis as u32 {
        return Ok(StyleChange::SizeAxis {
            axis: rec.number as u8,
            size: axis_size(rec.number2 as i32, rec.number3 as f32)?,
        });
    }
    if field == StyleField::AxisSize as u32
        || field == StyleField::MinAxis as u32
        || field == StyleField::MaxAxis as u32
    {
        let width = axis_size(i32::from(rec.color[0]), value)?;
        let height = axis_size(i32::from(rec.color[1]), rec.number2 as f32)?;
        return Ok(match field {
            field if field == StyleField::MinAxis as u32 => StyleChange::MinAxis { width, height },
            field if field == StyleField::MaxAxis as u32 => StyleChange::MaxAxis { width, height },
            _ => StyleChange::AxisSize { width, height },
        });
    }
    if field == StyleField::Focusable as u32 {
        return Ok(StyleChange::Focusable(rec.number > 0.5));
    }
    if field == StyleField::DragClaim as u32 {
        return Ok(StyleChange::DragClaim(rec.number as i32));
    }
    if field == StyleField::TextLimit as u32 {
        return Ok(StyleChange::TextLimit(
            (rec.number >= 0.0).then_some(rec.number.round() as u32),
        ));
    }
    if field == StyleField::Panel as u32 {
        return Ok(StyleChange::Panel);
    }
    if field == StyleField::TextRuns as u32 {
        return Ok(StyleChange::TextRuns(crate::text_runs::parse(&rec.text)?));
    }
    Err(uicore::Error::BadNumber)
}

pub(super) fn apply(tree: &mut uicore::Tree, mutation: Mutation) -> Result<Applied, uicore::Error> {
    let mut applied = Applied {
        id: None,
        gestures: Vec::new(),
        retired: Vec::new(),
    };
    match mutation {
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
            width,
            height,
        } => {
            tree.set_image(id, &source, width, height)?;
        }
        Mutation::EditText { id, edit } => {
            tree.text_edit(id, edit)?;
        }
        Mutation::Reparent { from, to } => tree.reparent_children(from, to)?,
        Mutation::Recognizers {
            id,
            tap,
            long_press,
            drag,
            window_ms,
        } => {
            tree.set_recognizers(id, recognizers(tap, long_press, drag, window_ms))?;
        }
        Mutation::Style { id, change } => match change {
            StyleChange::Focusable(enabled) => {
                applied.gestures = tree.set_focusable(id, enabled)?;
            }
            change => apply_style(tree, id, change)?,
        },
    }
    Ok(applied)
}

fn create_node(tree: &mut uicore::Tree, create: Create) -> Result<NodeId, uicore::Error> {
    match create {
        Create::Column { gap } => tree.column(gap),
        Create::Row { gap } => tree.row(gap),
        Create::Stack => tree.stack(),
        Create::Padding { edges } => tree.padding(edges),
        Create::Sized { width, height } => tree.sized(width, height),
        Create::Scroll { axis } => tree.scroll_with(uicore::ScrollAxis::from_code(axis)),
        Create::Align(alignment) => tree.align(alignment),
        Create::Grid { columns, rows, gap } => tree.grid_tracks(&columns, &rows, gap),
        Create::Pointer => tree.pointer_region(),
        Create::Decorated { color, radius } => tree.decorated(color, radius),
        Create::Clip { radius } => tree.clip(radius),
        Create::Opacity { alpha } => tree.opacity(alpha),
        Create::Transform => tree.transform(),
        Create::Image {
            source,
            width,
            height,
        } => tree.image(source, width, height),
        Create::Text {
            utf8,
            font_size,
            color,
        } => tree.text(utf8, font_size, color),
    }
}

fn apply_style(
    tree: &mut uicore::Tree,
    id: NodeId,
    change: StyleChange,
) -> Result<(), uicore::Error> {
    match change {
        StyleChange::Width(value) => {
            tree.set_axis_size(id, AxisSize::Points(value), AxisSize::Auto)
        }
        StyleChange::Height(value) => {
            tree.set_axis_size(id, AxisSize::Auto, AxisSize::Points(value))
        }
        StyleChange::Size { width, height } => tree.set_size(id, width, height),
        StyleChange::Padding(edges) => tree.set_padding(id, edges),
        StyleChange::PaddingEdges(edges) => tree.set_padding_edges(id, edges),
        StyleChange::PaddingEdge { edge, value } => {
            tree.set_padding_edge(id, box_edge(edge), value)
        }
        StyleChange::Margin(edges) => tree.set_margin(id, edges),
        StyleChange::Gap(gap) => tree.set_gap(id, gap),
        StyleChange::Grow(grow) => tree.set_grow(id, grow),
        StyleChange::Shrink(shrink) => tree.set_shrink(id, shrink),
        StyleChange::Basis(basis) => tree.set_basis(id, basis),
        StyleChange::MinSize { width, height } => tree.set_min_size(id, width, height),
        StyleChange::MaxSize { width, height } => tree.set_max_size(id, width, height),
        StyleChange::AlignSelf(alignment) => tree.set_align_self(id, alignment),
        StyleChange::ContainerAlign(alignment) => tree.set_align(id, alignment),
        StyleChange::Justify(justify) => tree.set_justify(id, justify),
        StyleChange::Wrap(wrap) => tree.set_wrap(id, wrap),
        StyleChange::Inset(inset) => tree.set_inset(id, inset),
        StyleChange::InsetEdge { edge, value } => tree.set_inset_edge(id, box_edge(edge), value),
        StyleChange::Grid => tree.set_grid_placement(
            id,
            GridLine::Auto,
            GridLine::Auto,
            GridLine::Auto,
            GridLine::Auto,
        ),
        StyleChange::GridLine { edge, line } => tree.set_grid_line(id, edge, line),
        StyleChange::FontSize(size) => tree.set_font_size(id, size),
        StyleChange::TextAlign(align) => tree.set_text_align(id, align),
        StyleChange::Weight(weight) => tree.set_font_weight(id, weight),
        StyleChange::MaxLines(lines) => tree.set_max_lines(id, lines),
        StyleChange::LineHeight(height) => tree.set_line_height(id, height),
        StyleChange::Color(color) => tree.set_color(id, color),
        StyleChange::Radius(radius) => tree.set_radius(id, radius),
        StyleChange::Stroke { width, color } => tree.set_stroke(id, width, color),
        StyleChange::Alpha(alpha) => tree.set_alpha(id, alpha),
        StyleChange::Blur(sigma) => tree.set_blur(id, sigma),
        StyleChange::Family(family) => tree.set_family(id, family.as_deref()),
        StyleChange::Ellipsis(ellipsis) => tree.set_ellipsis(id, ellipsis.as_deref()),
        StyleChange::SoftWrap(wrap) => tree.set_soft_wrap(id, wrap),
        StyleChange::Overflow(code) => tree.set_overflow(id, text_overflow(code)),
        StyleChange::Direction(rtl) => tree.set_text_direction(id, rtl),
        StyleChange::Reverse(reverse) => tree.set_reverse(id, reverse),
        StyleChange::Position(absolute) => tree.set_position(id, absolute),
        StyleChange::Aspect(ratio) => tree.set_aspect_ratio(id, ratio),
        StyleChange::Cross(align) => tree.set_row_cross(id, align),
        StyleChange::FlexFill => tree.set_fill(id),
        StyleChange::ScrollOffset { x, y } => tree.set_scroll_offset(id, x, y),
        StyleChange::ScrollAxis(axis) => {
            tree.set_scroll_axis(id, uicore::ScrollAxis::from_code(axis))
        }
        StyleChange::ScrollChain(chain) => tree.set_scroll_chain(id, chain),
        StyleChange::ImageMode { mode, required } => {
            tree.set_image_mode(id, content_mode(mode)?, required)
        }
        StyleChange::ScaleXY { x, y } => tree.set_scale_xy(id, x, y),
        StyleChange::Transform {
            x,
            y,
            rotate,
            scale,
        } => tree.set_transform(id, x, y, rotate, scale),
        StyleChange::TransformPart { component, value } => {
            tree.set_transform_component(id, component, value)
        }
        StyleChange::SizeAxis { axis, size } => tree.set_size_axis(id, axis, size),
        StyleChange::AxisSize { width, height } => tree.set_axis_size(id, width, height),
        StyleChange::MinAxis { width, height } => tree.set_min_axis(id, width, height),
        StyleChange::MaxAxis { width, height } => tree.set_max_axis(id, width, height),
        StyleChange::Focusable(_) => Ok(()),
        StyleChange::DragClaim(claim) => tree.set_drag_claim(id, drag_claim(claim)),
        StyleChange::TextLimit(limit) => tree.set_text_limit(id, limit),
        StyleChange::Panel => tree.set_panel(id),
        StyleChange::TextRuns(spans) => tree.set_text_spans(id, spans),
    }
}

pub(super) fn apply_now(inner: &Inner, mutation: Mutation) -> Result<Applied, uicore::Error> {
    let applied = {
        let mut tree = inner.tree.borrow_mut();
        apply(&mut tree, mutation)?
    };
    if let Err(message) = dispatch_gestures(inner, applied.gestures.clone()) {
        (inner.log)(3, &message);
    }
    forget(inner, &applied.retired);
    Ok(applied)
}

pub(super) fn flush_mutations(inner: &Inner) -> Result<bool, uicore::Error> {
    let pending = std::mem::take(&mut *inner.mutations.borrow_mut());
    if pending.is_empty() {
        return Ok(false);
    }
    for rec in pending {
        let mutation = parse(&rec)?;
        apply_now(inner, mutation)?;
    }
    Ok(true)
}

fn forget(inner: &Inner, ids: &[NodeId]) {
    if ids.is_empty() || inner.host.is_null() {
        return;
    }
    let raw: Vec<u64> = ids.iter().copied().map(NodeId::to_raw).collect();
    // SAFETY: `raw` lives until this call returns. `host` is the open interpreter.
    unsafe { qjs_forget_ids(inner.host, raw.as_ptr(), raw.len()) };
}

pub(super) fn owned_from_ptr(rec: &MutRec) -> Result<OwnedRec, ()> {
    let text = if rec.text.is_null() {
        String::new()
    } else {
        // SAFETY: `text` and `text_len` describe the bytes of this call.
        let Some(text) = (unsafe { utf8_of(rec.text, rec.text_len) }) else {
            return Err(());
        };
        text.to_owned()
    };
    Ok(OwnedRec {
        op: rec.op,
        field: rec.field,
        id: rec.id,
        parent: rec.parent,
        before: rec.before,
        number: rec.number,
        number2: rec.number2,
        number3: rec.number3,
        number4: rec.number4,
        color: rec.color,
        text,
    })
}

pub(super) unsafe extern "C" fn mutate(
    session: *mut c_void,
    rec: *const MutRec,
    out_id: *mut u64,
) -> i32 {
    super::qjs_entry!(-1, {
        if rec.is_null() {
            return -1;
        }
        let inner = unsafe { inner(session) };
        // SAFETY: glue passes a `MutRec` that lives for this call.
        let owned = match owned_from_ptr(unsafe { &*rec }) {
            Ok(owned) => owned,
            Err(()) => return -1,
        };
        let mutation = match parse(&owned) {
            Ok(mutation) => mutation,
            Err(_) => return -1,
        };
        match apply_now(inner, mutation) {
            Ok(applied) => {
                if !out_id.is_null() {
                    let raw = applied.id.map(NodeId::to_raw).unwrap_or(0);
                    // SAFETY: glue passes a writable `uint64_t` when it wants the id.
                    unsafe { *out_id = raw };
                }
                0
            }
            Err(_) => -1,
        }
    })
}

pub(super) unsafe extern "C" fn push_mutation(
    session: *mut c_void,
    op: u32,
    id: u64,
    parent: u64,
    before: u64,
    field: u32,
    number: f64,
    number2: f64,
    color: *const u8,
    text: *const c_char,
    text_len: usize,
) -> i32 {
    super::qjs_entry!(-1, {
        let inner = unsafe { inner(session) };
        let text = if text.is_null() {
            String::new()
        } else {
            let Some(text) = (unsafe { utf8_of(text, text_len) }) else {
                return -1;
            };
            text.to_owned()
        };
        let color = unsafe { super::rgba(color) }.unwrap_or([0, 0, 0, 0]);
        let rec =
            OwnedRec::from_queued(op, id, parent, before, field, number, number2, color, text);
        if parse(&rec).is_err() {
            return -1;
        }
        inner.mutations.borrow_mut().push(rec);
        0
    })
}

/// `rect` reads geometry, so a queued style write has to land first.
pub(super) fn flush_for_read(inner: &Inner) -> i32 {
    match flush_mutations(inner) {
        Ok(true) => {
            let mut tree = inner.tree.borrow_mut();
            if tree.needs_layout() && tree.layout().is_err() {
                return -1;
            }
            0
        }
        Ok(false) => 0,
        Err(_) => -1,
    }
}

fn edges_of(rec: &OwnedRec) -> Edges {
    Edges {
        top: rec.number as f32,
        right: rec.number2 as f32,
        bottom: rec.number3 as f32,
        left: rec.number4 as f32,
    }
}

fn inset_of(rec: &OwnedRec) -> Inset {
    Inset {
        top: edge_auto(rec.color[0], rec.number as f32),
        right: edge_auto(rec.color[1], rec.number2 as f32),
        bottom: edge_auto(rec.color[2], rec.number3 as f32),
        left: edge_auto(rec.color[3], rec.number4 as f32),
    }
}

fn edge_auto(tag: u8, value: f32) -> Option<f32> {
    if tag == 0 { Some(value) } else { None }
}

fn split_tracks(text: &str) -> Result<(&str, &str), uicore::Error> {
    text.split_once('|').ok_or(uicore::Error::BadNumber)
}

fn alignment(value: u32) -> Result<Alignment, uicore::Error> {
    match value {
        0 => Ok(Alignment::Start),
        1 => Ok(Alignment::Center),
        2 => Ok(Alignment::End),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn justify(value: u32) -> Result<Justify, uicore::Error> {
    match value {
        0 => Ok(Justify::Start),
        1 => Ok(Justify::Center),
        2 => Ok(Justify::End),
        3 => Ok(Justify::SpaceBetween),
        4 => Ok(Justify::SpaceAround),
        5 => Ok(Justify::SpaceEvenly),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn cross(value: u32) -> Result<RowCrossAlign, uicore::Error> {
    match value {
        0 => Ok(RowCrossAlign::Start),
        1 => Ok(RowCrossAlign::Center),
        2 => Ok(RowCrossAlign::End),
        3 => Ok(RowCrossAlign::Stretch),
        4 => Ok(RowCrossAlign::Baseline),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn text_align(value: u32) -> TextAlign {
    match value {
        1 => TextAlign::Center,
        2 => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

fn box_edge(edge: u8) -> uicore::BoxEdge {
    match edge {
        0 => uicore::BoxEdge::Top,
        1 => uicore::BoxEdge::Right,
        2 => uicore::BoxEdge::Bottom,
        _ => uicore::BoxEdge::Left,
    }
}

fn axis_size(tag: i32, value: f32) -> Result<AxisSize, uicore::Error> {
    match tag {
        0 => Ok(AxisSize::Auto),
        1 => Ok(AxisSize::Points(value)),
        2 => Ok(AxisSize::Fraction(value)),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn grid_line(kind: i32, value: i32) -> Result<GridLine, uicore::Error> {
    match kind {
        0 => Ok(GridLine::Auto),
        1 => i16::try_from(value)
            .map(GridLine::Line)
            .map_err(|_| uicore::Error::BadNumber),
        2 => u16::try_from(value.max(0))
            .map(GridLine::Span)
            .map_err(|_| uicore::Error::BadNumber),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn alpha_byte(value: f64) -> Result<u8, uicore::Error> {
    if !(0.0..=255.0).contains(&value) || value.fract() != 0.0 {
        return Err(uicore::Error::BadNumber);
    }
    Ok(value as u8)
}

fn text_overflow(code: i32) -> protocol::TextOverflow {
    match code {
        1 => protocol::TextOverflow::Ellipsis,
        2 => protocol::TextOverflow::Visible,
        _ => protocol::TextOverflow::Clip,
    }
}

fn content_mode(mode: i32) -> Result<protocol::ContentMode, uicore::Error> {
    match mode {
        0 => Ok(protocol::ContentMode::Fill),
        1 => Ok(protocol::ContentMode::Fit),
        2 => Ok(protocol::ContentMode::Crop),
        _ => Err(uicore::Error::BadNumber),
    }
}

fn drag_claim(claim: i32) -> uicore::DragClaim {
    match claim {
        1 => uicore::DragClaim::Horizontal,
        2 => uicore::DragClaim::Free,
        _ => uicore::DragClaim::None,
    }
}

fn recognizers(tap: i32, long_press: i32, drag: i32, window_ms: f64) -> uicore::RecognizerSet {
    let mut set = uicore::RecognizerSet::default();
    set.tap = tap != 0;
    set.long_press = long_press != 0;
    set.drag = match drag {
        0 => Some(uicore::DragAxis::Free),
        1 => Some(uicore::DragAxis::Horizontal),
        2 => Some(uicore::DragAxis::Vertical),
        _ => None,
    };
    if window_ms > 0.0 {
        set.double_tap = Some(uicore::DoubleTapConfig {
            slop: uicore::DRAG_SLOP,
            window_ms,
        });
    }
    set
}
