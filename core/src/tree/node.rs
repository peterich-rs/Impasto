//! Node identity, gesture names, and the private node payload.

use std::cell::Cell;
use std::sync::Arc;

use protocol::TextStyle;
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

pub(super) const MAX_GENERATION: u32 = (1 << 21) - 1;

/// A node in one [`Tree`]. The slot index is stable. `generation` changes when
/// the slot is recycled, so a stale id cannot address the node that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(super) index: u32,
    pub(super) generation: u32,
}

impl NodeId {
    pub fn to_raw(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.index)
    }

    pub fn from_raw(raw: u64) -> Self {
        Self {
            index: raw as u32,
            generation: (raw >> 32) as u32,
        }
    }

    pub(super) fn slot(self) -> usize {
        self.index as usize
    }
}

/// How a pointer event changed a [`Tree::pointer_region`], a scroll, or focus.
///
/// `DragBy` is a scene-point delta. `DragEnd`'s pair is velocity in scene points
/// per millisecond. A locked drag emits `Release` for the pressed region, then
/// `DragBegin`, then `DragBy` samples, then `DragEnd`. A click moves focus:
/// `Blur` of the previous node, then `Focus` of the focusable ancestor, when
/// that pair actually changes. A drag that locks the scroll does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Press(NodeId),
    Release(NodeId),
    Click(NodeId),
    DragBegin(NodeId),
    DragBy(NodeId, f32, f32),
    DragEnd(NodeId, f32, f32),
    Focus(NodeId),
    Blur(NodeId),
    Hover(NodeId),
    Leave(NodeId),
    /// Wheel deltas in scene points. The id is the nearest scroll ancestor.
    Wheel(NodeId, f32, f32),
    /// Right click on a pointer region. No focus change.
    SecondaryClick(NodeId),
    /// A scroll offset changed in core. The pair is the new offset in scene points.
    Scroll(NodeId, f32, f32),
    /// Accessibility loaded a new editor string. The text stays on the editor.
    TextReplaced(NodeId),
    /// A span link was activated. `link` is the UTF-16 range's id.
    ActivateLink {
        id: NodeId,
        link: u32,
    },
    /// The long-press recognizer won. A later lift is `Release` and not `Click`.
    LongPress(NodeId),
    /// Second tap inside the double-tap window. The id is the pressed node.
    DoubleTap(NodeId),
    /// Magnification routed to the deepest transform. Core does not change the matrix.
    Zoom {
        id: NodeId,
        scale: f32,
    },
    /// A scroll's phase changed. Velocity is points per millisecond; positive increases offset.
    ScrollPhaseChanged {
        id: NodeId,
        phase: crate::ScrollPhase,
        vx: f32,
        vy: f32,
        ox: f32,
        oy: f32,
    },
}

/// Which drags a pointer region takes before a scroll does.
///
/// `Horizontal` is a single-line field: a vertical move still scrolls.
/// `Free` is a multi-line field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DragClaim {
    #[default]
    None,
    Horizontal,
    Free,
}

/// Where [`Tree::focus_step`] moves the text focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
}

/// Ids retired by [`Tree::discard`], children before their parent, plus any
/// focus gesture that retirement produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Discarded {
    pub ids: Vec<NodeId>,
    pub gestures: Vec<Gesture>,
}

/// Main-axis distribution for a row or column. Matches CSS `justify-content`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// How a child sits inside an [`Tree::align`] node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

/// Cross-axis alignment of a row or column. Baseline applies to a row only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowCrossAlign {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// Which edge [`Tree::scroll_into_view`] aligns to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollEdge {
    Start,
    End,
    /// The smallest move that brings the node fully into the viewport.
    Nearest,
}

/// Four edges in scene points. [`Edges::all`] is the uniform padding the older API writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub fn all(value: f32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// A main size written onto Taffy's `Style::size`.
///
/// `Fraction(0.5)` is 50% of the parent. Values above 1 are allowed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisSize {
    Auto,
    Points(f32),
    Fraction(f32),
}

/// Insets in scene points. `None` is the auto edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inset {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

/// One edge of a grid placement. `Line(0)` and `Span(0)` are rejected.
///
/// A negative line counts from the end, the same way Taffy does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridLine {
    Auto,
    Line(i16),
    Span(u16),
}

/// One grid track. `Points` and `Fraction` must be finite and non-negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackSize {
    MinContent,
    MaxContent,
    Points(f32),
    Fraction(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Row,
    Column,
    Grid,
    Padding,
    Align,
    Sized,
    Stack,
    Clip,
    Opacity,
    Scroll,
    Transform,
    Text,
    Image,
    PointerRegion,
    Decorated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    /// The box itself is a target when no child takes the point.
    Opaque,
    /// Only children can be hit.
    Defer,
    /// The box is a click target. Children are tested first.
    Region,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub(super) struct NodeFlags: u32 {
        const LAYOUT_DIRTY = 1 << 0;
        const BOUNDARY_DIRTY = 1 << 1;
        const REPAINT_DIRTY = 1 << 2;
        const SEMANTICS_DIRTY = 1 << 3;
        const RELAYOUT_BOUNDARY = 1 << 4;
        const REPAINT_BOUNDARY = 1 << 5;
        const FOCUSABLE = 1 << 6;
        /// Row children share a text baseline. Column ignores this flag.
        const BASELINE_CROSS = 1 << 7;
        /// Text paint identity changed and [`super::layout::Tree::sync_paragraphs`]
        /// has not consumed it yet.
        const TEXT_DIRTY = 1 << 8;
    }
}

pub(super) enum Payload {
    Container,
    Text(Box<TextPayload>),
    Image(ImagePayload),
}

pub(super) struct TextPayload {
    pub(super) utf8: Arc<str>,
    pub(super) font_size: f32,
    pub(super) color: [u8; 4],
    /// Paragraph constraint passed to [`crate::Shaper::metrics`] for this layout.
    pub(super) max_width: Cell<f32>,
    pub(super) style: Arc<TextStyle>,
    /// Empty means one run with `color` and `style`. Non-empty runs concatenate to `utf8`.
    pub(super) spans: Arc<[protocol::TextSpan]>,
    /// UTF-16 ranges. A click whose caret falls in one emits `ActivateLink`.
    pub(super) links: Arc<[protocol::TextLink]>,
    /// Present while the node is being edited. The tree does not keep a second map.
    pub(super) editor: Option<crate::TextEditor>,
    /// Line boxes for the current metrics key. Color changes leave this in place.
    pub(super) metrics: Option<Arc<crate::ParagraphMetrics>>,
    /// Retained paint paragraph. Released when the paint key changes or the node is retired.
    pub(super) paragraph: Option<protocol::ParagraphId>,
    /// Last key passed to `retain_paint`.
    pub(super) cached: Option<crate::PaintKey>,
}

impl TextPayload {
    pub(super) fn new(utf8: impl Into<Arc<str>>, font_size: f32, color: [u8; 4]) -> Self {
        Self {
            utf8: utf8.into(),
            font_size,
            color,
            max_width: Cell::new(0.0),
            style: Arc::new(TextStyle::default()),
            spans: Arc::from([]),
            links: Arc::from([]),
            editor: None,
            metrics: None,
            paragraph: None,
            cached: None,
        }
    }

    pub(super) fn style_mut(&mut self) -> &mut TextStyle {
        Arc::make_mut(&mut self.style)
    }
}

pub(super) struct ImagePayload {
    pub(super) source: Arc<str>,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) mode: protocol::ContentMode,
    /// Missing required images fail the frame. The three-argument constructor is optional.
    pub(super) required: bool,
}

/// Absent when the node has no fill, stroke, shadow, blur, radius, alpha, or blend.
#[derive(Clone, Debug)]
pub(super) struct PaintProps {
    pub(super) fill: Option<[u8; 4]>,
    pub(super) corner_radius: f32,
    pub(super) stroke_width: f32,
    pub(super) stroke_color: [u8; 4],
    pub(super) gradient: Option<protocol::Gradient>,
    pub(super) shadow: Option<protocol::Shadow>,
    pub(super) box_shadows: Arc<[protocol::BoxShadow]>,
    pub(super) blur: f32,
    pub(super) backdrop: f32,
    pub(super) blend: protocol::BlendMode,
    pub(super) alpha: Option<u8>,
}

impl PaintProps {
    fn is_plain(&self) -> bool {
        self.fill.is_none()
            && self.corner_radius == 0.0
            && self.stroke_width <= 0.0
            && self.gradient.is_none()
            && self.shadow.is_none()
            && self.box_shadows.is_empty()
            && !(self.blur.is_finite() && self.blur > 0.0)
            && !(self.backdrop.is_finite() && self.backdrop > 0.0)
            && self.blend == protocol::BlendMode::SrcOver
            && self.alpha.is_none()
    }
}

impl Default for PaintProps {
    fn default() -> Self {
        Self {
            fill: None,
            corner_radius: 0.0,
            stroke_width: 0.0,
            stroke_color: [0, 0, 0, 0],
            gradient: None,
            shadow: None,
            box_shadows: Arc::from([]),
            blur: 0.0,
            backdrop: 0.0,
            blend: protocol::BlendMode::SrcOver,
            alpha: None,
        }
    }
}

pub(super) enum Geom {
    None,
    Scroll(ScrollGeom),
    Transform(TransformGeom),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ScrollGeom {
    pub(super) offset_x: f32,
    pub(super) offset_y: f32,
    pub(super) overscroll_x: f32,
    pub(super) overscroll_y: f32,
    pub(super) physics: crate::ScrollPhysics,
    pub(super) chain: bool,
    pub(super) phase: crate::ScrollPhase,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TransformGeom {
    pub(super) translate_x: f32,
    pub(super) translate_y: f32,
    pub(super) rotate_rad: f32,
    pub(super) scale_x: f32,
    pub(super) scale_y: f32,
}

impl TransformGeom {
    fn identity() -> Self {
        Self {
            translate_x: 0.0,
            translate_y: 0.0,
            rotate_rad: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
        }
    }

    pub(super) fn is_identity(&self) -> bool {
        self.translate_x == 0.0
            && self.translate_y == 0.0
            && self.rotate_rad == 0.0
            && self.scale_x == 1.0
            && self.scale_y == 1.0
    }
}

pub(super) struct Node {
    pub(super) kind: Kind,
    pub(super) payload: Payload,
    pub(super) paint: Option<Box<PaintProps>>,
    pub(super) geom: Geom,
    /// Copied at creation. Revive and the setters read this, not the live Taffy style.
    pub(super) style: Style,
    /// `None` after the subtree's Taffy nodes were released. The scene node stays.
    pub(super) taffy: Option<TaffyId>,
    pub(super) parent: Option<NodeId>,
    pub(super) first_child: Option<NodeId>,
    pub(super) last_child: Option<NodeId>,
    pub(super) next: Option<NodeId>,
    pub(super) prev: Option<NodeId>,
    /// Parent-relative border position. `Tree::rect` adds the viewport origin.
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) flags: NodeFlags,
    /// Wins a drag from this region before the scroll chain does.
    pub(super) drag_claim: DragClaim,
    pub(super) recognizers: crate::RecognizerSet,
    pub(super) semantics: Option<crate::SemanticsHint>,
}

impl Node {
    pub(super) fn new(kind: Kind, style: Style, taffy: TaffyId, payload: Payload) -> Self {
        let mut flags = NodeFlags::LAYOUT_DIRTY | NodeFlags::SEMANTICS_DIRTY;
        if matches!(kind, Kind::Scroll | Kind::Sized) {
            flags.insert(NodeFlags::RELAYOUT_BOUNDARY);
        }
        if matches!(
            kind,
            Kind::Scroll | Kind::Transform | Kind::Opacity | Kind::Clip
        ) {
            flags.insert(NodeFlags::REPAINT_BOUNDARY);
        }
        let geom = match kind {
            Kind::Scroll => Geom::Scroll(ScrollGeom {
                offset_x: 0.0,
                offset_y: 0.0,
                overscroll_x: 0.0,
                overscroll_y: 0.0,
                physics: crate::ScrollPhysics::default(),
                chain: true,
                phase: crate::ScrollPhase::Idle,
            }),
            Kind::Transform => Geom::Transform(TransformGeom::identity()),
            _ => Geom::None,
        };
        Self {
            kind,
            payload,
            paint: None,
            geom,
            style,
            taffy: Some(taffy),
            parent: None,
            first_child: None,
            last_child: None,
            next: None,
            prev: None,
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            flags,
            drag_claim: DragClaim::None,
            recognizers: crate::RecognizerSet::default(),
            semantics: None,
        }
    }

    pub(super) fn text(&self) -> Option<&TextPayload> {
        match &self.payload {
            Payload::Text(text) => Some(text),
            Payload::Container | Payload::Image(_) => None,
        }
    }

    pub(super) fn text_mut(&mut self) -> Option<&mut TextPayload> {
        match &mut self.payload {
            Payload::Text(text) => Some(text),
            Payload::Container | Payload::Image(_) => None,
        }
    }

    pub(super) fn image(&self) -> Option<&ImagePayload> {
        match &self.payload {
            Payload::Image(image) => Some(image),
            Payload::Container | Payload::Text(_) => None,
        }
    }

    pub(super) fn image_mut(&mut self) -> Option<&mut ImagePayload> {
        match &mut self.payload {
            Payload::Image(image) => Some(image),
            Payload::Container | Payload::Text(_) => None,
        }
    }

    /// True when this node's subtree must composite through its own layer.
    ///
    /// A full-alpha opacity and an identity transform draw straight into the
    /// parent picture, the way a settled `FadeTransition` or a parked page does.
    pub(super) fn composites(&self) -> bool {
        let live = |sigma: f32| sigma.is_finite() && sigma > 0.0;
        match self.kind {
            Kind::Scroll => true,
            Kind::Transform => !self.transform().is_identity(),
            Kind::Opacity => self.alpha().unwrap_or(255) != 255,
            Kind::Clip => false,
            _ => {
                live(self.blur())
                    || live(self.backdrop())
                    || self.blend() != protocol::BlendMode::SrcOver
            }
        }
    }

    /// Blur, backdrop blur, or a blend other than [`protocol::BlendMode::SrcOver`].
    pub(super) fn paints_effect(&self) -> bool {
        let live = |sigma: f32| sigma.is_finite() && sigma > 0.0;
        live(self.blur()) || live(self.backdrop()) || self.blend() != protocol::BlendMode::SrcOver
    }

    /// This node opens layers and owns the runs under it this frame.
    ///
    /// `subtree` is true when a strict descendant opens layers. A clip uses that
    /// to decide whether its bracket is a layer or commands inside the parent run.
    /// Alpha 0 and a full alpha do not open an opacity layer. An identity transform
    /// does not open a transform layer. Effects do, on any kind.
    pub(super) fn renders_layers(&self, subtree: bool) -> bool {
        if self.paints_effect() {
            return true;
        }
        if self.kind == Kind::Clip {
            return subtree;
        }
        // Alpha 0 skips the subtree. It does not open an opacity layer.
        if self.kind == Kind::Opacity && self.alpha() == Some(0) {
            return false;
        }
        self.composites()
    }

    /// Scroll keeps one child picture. Other layers keep runs.
    pub(super) fn caches_picture(&self) -> bool {
        self.kind == Kind::Scroll
    }

    pub(super) fn hit_role(&self) -> Hit {
        match self.kind {
            Kind::Text | Kind::Image => Hit::Opaque,
            Kind::PointerRegion => Hit::Region,
            Kind::Row
            | Kind::Column
            | Kind::Grid
            | Kind::Padding
            | Kind::Align
            | Kind::Sized
            | Kind::Stack
            | Kind::Clip
            | Kind::Opacity
            | Kind::Scroll
            | Kind::Transform
            | Kind::Decorated => Hit::Defer,
        }
    }

    pub(super) fn is_focusable(&self) -> bool {
        self.flags.contains(NodeFlags::FOCUSABLE)
    }

    pub(super) fn set_focusable_flag(&mut self, focusable: bool) {
        self.flags.set(NodeFlags::FOCUSABLE, focusable);
    }

    pub(super) fn write_paint(&mut self, edit: impl FnOnce(&mut PaintProps)) {
        let paint = self
            .paint
            .get_or_insert_with(|| Box::new(PaintProps::default()));
        edit(paint);
        if paint.is_plain() {
            self.paint = None;
        }
    }

    pub(super) fn fill(&self) -> Option<[u8; 4]> {
        self.paint.as_ref().and_then(|paint| paint.fill)
    }

    pub(super) fn corner_radius(&self) -> f32 {
        self.paint
            .as_ref()
            .map(|paint| paint.corner_radius)
            .unwrap_or(0.0)
    }

    pub(super) fn alpha(&self) -> Option<u8> {
        self.paint.as_ref().and_then(|paint| paint.alpha)
    }

    pub(super) fn stroke_width(&self) -> f32 {
        self.paint
            .as_ref()
            .map(|paint| paint.stroke_width)
            .unwrap_or(0.0)
    }

    pub(super) fn stroke_color(&self) -> [u8; 4] {
        self.paint
            .as_ref()
            .map(|paint| paint.stroke_color)
            .unwrap_or([0, 0, 0, 0])
    }

    pub(super) fn gradient(&self) -> Option<&protocol::Gradient> {
        self.paint
            .as_ref()
            .and_then(|paint| paint.gradient.as_ref())
    }

    pub(super) fn box_shadows(&self) -> Arc<[protocol::BoxShadow]> {
        self.paint
            .as_ref()
            .map(|paint| Arc::clone(&paint.box_shadows))
            .unwrap_or_else(|| Arc::from([]))
    }

    pub(super) fn shadow(&self) -> Option<protocol::Shadow> {
        self.paint.as_ref().and_then(|paint| paint.shadow)
    }

    pub(super) fn blur(&self) -> f32 {
        self.paint.as_ref().map(|paint| paint.blur).unwrap_or(0.0)
    }

    pub(super) fn backdrop(&self) -> f32 {
        self.paint
            .as_ref()
            .map(|paint| paint.backdrop)
            .unwrap_or(0.0)
    }

    pub(super) fn blend(&self) -> protocol::BlendMode {
        self.paint
            .as_ref()
            .map(|paint| paint.blend)
            .unwrap_or(protocol::BlendMode::SrcOver)
    }

    pub(super) fn scroll_offset(&self) -> (f32, f32) {
        match self.geom {
            Geom::Scroll(geom) => (geom.offset_x, geom.offset_y),
            Geom::None | Geom::Transform(_) => (0.0, 0.0),
        }
    }

    /// Offset plus rubber-band. Record and hit testing use this pair.
    pub(super) fn scroll_paint(&self) -> (f32, f32) {
        match self.geom {
            Geom::Scroll(geom) => (
                geom.offset_x + geom.overscroll_x,
                geom.offset_y + geom.overscroll_y,
            ),
            Geom::None | Geom::Transform(_) => (0.0, 0.0),
        }
    }

    pub(super) fn overscroll(&self) -> (f32, f32) {
        match self.geom {
            Geom::Scroll(geom) => (geom.overscroll_x, geom.overscroll_y),
            Geom::None | Geom::Transform(_) => (0.0, 0.0),
        }
    }

    pub(super) fn scroll_geom(&self) -> Option<&ScrollGeom> {
        match &self.geom {
            Geom::Scroll(geom) => Some(geom),
            Geom::None | Geom::Transform(_) => None,
        }
    }

    pub(super) fn scroll_mut(&mut self) -> Option<&mut ScrollGeom> {
        match &mut self.geom {
            Geom::Scroll(geom) => Some(geom),
            Geom::None | Geom::Transform(_) => None,
        }
    }

    pub(super) fn transform(&self) -> TransformGeom {
        match self.geom {
            Geom::Transform(geom) => geom,
            Geom::None | Geom::Scroll(_) => TransformGeom::identity(),
        }
    }

    pub(super) fn transform_mut(&mut self) -> Option<&mut TransformGeom> {
        match &mut self.geom {
            Geom::Transform(geom) => Some(geom),
            Geom::None | Geom::Scroll(_) => None,
        }
    }
}

pub(super) struct Slot {
    pub(super) generation: u32,
    pub(super) node: Option<Node>,
}
