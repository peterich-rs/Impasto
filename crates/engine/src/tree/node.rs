//! Node identity, gesture names, and the private node payload.

use std::cell::Cell;
use std::sync::Arc;

use crate::geometry::NodeId;
use crate::scene::TextStyle;
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

/// How a pointer event changed a [`crate::tree::Tree::pointer_region`], a scroll, or focus.
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
        phase: crate::motion::ScrollPhase,
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

/// Where [`crate::tree::Tree::focus_step`] moves the text focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
}

/// Ids retired by [`crate::tree::Tree::discard`], children before their parent, plus any
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
/// How a child sits inside an [`crate::tree::Tree::align`] node.
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

/// Which edge [`crate::tree::Tree::scroll_into_view`] aligns to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollEdge {
    Start,
    End,
    /// The smallest move that brings the node fully into the viewport.
    Nearest,
}

/// One side of a box. Used by the edge setters so a partial update does not need a second store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxEdge {
    Top,
    Right,
    Bottom,
    Left,
}

/// Where [`crate::tree::Tree::place_panel`] puts a measured panel relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelAlign {
    BelowStart,
    BelowEnd,
    BelowCenter,
    AboveStart,
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

/// Available space for a subtree measurement, the panel flavor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SubtreeSpace {
    /// Natural size on both axes.
    MaxContent,
    /// Natural size, but when the width exceeds the cap, re-lay out with the
    /// cap as a definite width.
    CappedMax(f32),
    /// A definite width; the height may stay natural.
    Definite { width: f32, height: Option<f32> },
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

/// Layout identity. Row, column, padding, and the other old tags are presets
/// of [`Kind::View`]. Clip, opacity, and transform are a [`Kind::Stack`] plus
/// a [`Role`] flag or a paint field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    View,
    Stack,
    Scroll,
    Text,
    Image,
}

bitflags::bitflags! {
    /// Capabilities that used to be their own node kind.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub(super) struct Role: u32 {
        const CLIP = 1 << 0;
        const POINTER = 1 << 1;
        /// The node was created as a transform, even while the matrix is identity.
        const TRANSFORM = 1 << 2;
        /// Author grid. A stack's single cell does not set this.
        const GRID = 1 << 3;
    }
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
        /// Text metrics changed and [`super::text::Tree::sync_metrics`]
        /// has not consumed it yet.
        const TEXT_DIRTY = 1 << 8;
        /// Skip paint and hit testing. Layout size stays, so a pinned list row
        /// does not change the prefix sums.
        const CULL_PAINT = 1 << 9;
        /// List-row picture. A scroll does not fold this subtree into its child
        /// picture, so a text edit rebuilds only the row. Canvas ownership is
        /// [`Self::PICTURE_FROM_CANVAS`]; either bit makes a picture owner.
        const PICTURE_OWNER = 1 << 10;
        /// Canvas list installed on this node. Independent of [`Self::PICTURE_OWNER`].
        const PICTURE_FROM_CANVAS = 1 << 11;
        /// A shader owns its picture so scalar tracks can address it independently.
        const PICTURE_FROM_SHADER = 1 << 12;
        /// `display: none`. Layout, paint, hit, and semantics skip the subtree.
        const HIDDEN = 1 << 13;
        /// Keep the node drawable while omitting its subtree from accessibility.
        const SEMANTICS_HIDDEN = 1 << 14;
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
    /// Paragraph constraint passed to [`crate::text::Shaper::metrics`] for this layout.
    pub(super) max_width: Cell<f32>,
    pub(super) style: Arc<TextStyle>,
    /// Empty means one run with `color` and `style`. Non-empty runs concatenate to `utf8`.
    pub(super) spans: Arc<[crate::scene::TextSpan]>,
    /// UTF-16 ranges. A click whose caret falls in one emits `ActivateLink`.
    pub(super) links: Arc<[crate::tree::TextLink]>,
    /// Present while the node is being edited. The tree does not keep a second map.
    pub(super) editor: Option<crate::text::TextEditor>,
    /// Line boxes for the current metrics key. Color changes leave this in place.
    pub(super) metrics: Option<Arc<crate::text::ParagraphMetrics>>,
    /// Last geometry key measured for this node. Color is not part of the key.
    pub(super) cached: Option<crate::text::MetricsKey>,
    /// Foreground the frozen glyphs were last recorded with. A change repaints
    /// without a new shape.
    pub(super) painted: Option<PaintedForeground>,
}

/// The brushes a text node's current command carries.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PaintedForeground {
    pub(super) color: [u8; 4],
    pub(super) span_paints: Arc<[crate::scene::SpanPaint]>,
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
            cached: None,
            painted: None,
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
    pub(super) mode: crate::scene::ContentMode,
    /// Missing required images fail the frame. The three-argument constructor is optional.
    pub(super) required: bool,
}

/// Absent when the node has no fill, stroke, shadow, blur, radius, alpha, or blend.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PaintProps {
    pub(super) fill: Option<[u8; 4]>,
    pub(super) corner_radius: f32,
    pub(super) corner_radii: Option<[f32; 4]>,
    pub(super) corner_curve: f32,
    pub(super) stroke_width: f32,
    pub(super) stroke_color: [u8; 4],
    pub(super) outline_width: f32,
    pub(super) outline_color: [u8; 4],
    pub(super) gradient: Option<crate::scene::Gradient>,
    pub(super) shadow: Option<crate::scene::Shadow>,
    pub(super) box_shadows: Arc<[crate::scene::BoxShadow]>,
    pub(super) blur: f32,
    pub(super) backdrop: f32,
    pub(super) blend: crate::scene::BlendMode,
    pub(super) alpha: Option<u8>,
}

impl PaintProps {
    fn is_plain(&self) -> bool {
        self.fill.is_none()
            && self.corner_radius == 0.0
            && self.corner_radii.is_none()
            && self.corner_curve == 2.0
            && self.stroke_width <= 0.0
            && self.outline_width <= 0.0
            && self.gradient.is_none()
            && self.shadow.is_none()
            && self.box_shadows.is_empty()
            && !(self.blur.is_finite() && self.blur > 0.0)
            && !(self.backdrop.is_finite() && self.backdrop > 0.0)
            && self.blend == crate::scene::BlendMode::SrcOver
            && self.alpha.is_none()
    }
}

impl Default for PaintProps {
    fn default() -> Self {
        Self {
            fill: None,
            corner_radius: 0.0,
            corner_radii: None,
            corner_curve: 2.0,
            stroke_width: 0.0,
            stroke_color: [0, 0, 0, 0],
            outline_width: 0.0,
            outline_color: [0, 0, 0, 0],
            gradient: None,
            shadow: None,
            box_shadows: Arc::from([]),
            blur: 0.0,
            backdrop: 0.0,
            blend: crate::scene::BlendMode::SrcOver,
            alpha: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ScrollGeom {
    pub(super) offset_x: f32,
    pub(super) offset_y: f32,
    pub(super) overscroll_x: f32,
    pub(super) overscroll_y: f32,
    pub(super) physics: crate::motion::ScrollPhysics,
    pub(super) chain: bool,
    pub(super) phase: crate::motion::ScrollPhase,
    pub(super) axis: crate::motion::ScrollAxis,
    /// Floor added to the content height when computing limits.
    pub(super) content_min: f32,
    pub(super) pull_mode: crate::motion::PullMode,
    pub(super) pull_threshold: f32,
    /// Edge-mode accumulator. Overscroll mode uses [`ScrollGeom::pull_armed`].
    pub(super) pull: f32,
    pub(super) pull_armed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TransformGeom {
    pub(super) translate_x: f32,
    pub(super) translate_y: f32,
    pub(super) rotate_rad: f32,
    pub(super) scale_x: f32,
    pub(super) scale_y: f32,
    /// Scene pivot. `None` keeps the constructor origin at the canvas origin.
    /// `Some((points_x, points_y, ratio_x, ratio_y))` is the node border box.
    pub(super) pivot: Option<(f32, f32, f32, f32)>,
}

impl ScrollGeom {
    pub(super) fn fresh(axis: crate::motion::ScrollAxis) -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            overscroll_x: 0.0,
            overscroll_y: 0.0,
            physics: crate::motion::ScrollPhysics::default(),
            chain: true,
            phase: crate::motion::ScrollPhase::Idle,
            axis,
            content_min: 0.0,
            pull_mode: crate::motion::PullMode::Off,
            pull_threshold: 64.0,
            pull: 0.0,
            pull_armed: false,
        }
    }
}

impl TransformGeom {
    pub(super) fn identity() -> Self {
        Self {
            translate_x: 0.0,
            translate_y: 0.0,
            rotate_rad: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            pivot: None,
        }
    }

    /// Fold `pivot` into the single translate the draw command can carry.
    ///
    /// A missing pivot leaves the constructor matrix alone. Ratios are of this
    /// node's border box, whose top-left is `(x, y)` in the same space as the
    /// commands being transformed.
    pub(super) fn placed(&self, x: f32, y: f32, w: f32, h: f32) -> Self {
        let [translate_x, translate_y] = self.compositor_transform(x, y, w, h).placed();
        Self {
            translate_x,
            translate_y,
            pivot: None,
            ..*self
        }
    }

    pub(super) fn compositor_transform(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> crate::scene::CompositorTransform {
        crate::scene::CompositorTransform {
            translate: [self.translate_x, self.translate_y],
            rotate_rad: self.rotate_rad,
            scale: [self.scale_x, self.scale_y],
            pivot: self
                .pivot
                .map(|(ox, oy, rx, ry)| [x + ox + rx * w, y + oy + ry * h]),
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
    pub(super) role: Role,
    pub(super) payload: Payload,
    pub(super) paint: Option<Box<PaintProps>>,
    /// Installed canvas. Picture ownership is [`NodeFlags::PICTURE_FROM_CANVAS`].
    pub(super) canvas: Option<crate::scene::CanvasList>,
    /// Scroll state. Independent of [`Node::transform`], so a scroller can move.
    pub(super) scroll: Option<Box<ScrollGeom>>,
    pub(super) transform: Option<Box<TransformGeom>>,
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
    /// Border plus padding, so text and images share the content box.
    pub(super) content_x: f32,
    pub(super) content_y: f32,
    pub(super) content_w: f32,
    pub(super) content_h: f32,
    pub(super) flags: NodeFlags,
    /// Paint and hit order among siblings. Zero keeps the tree order.
    pub(super) z_index: i32,
    /// Wins a drag from this region before the scroll chain does.
    pub(super) drag_claim: DragClaim,
    pub(super) recognizers: crate::input::RecognizerSet,
    pub(super) semantics: Option<crate::semantics::SemanticsHint>,
}

macro_rules! payload_accessors {
    ($name:ident, $name_mut:ident, $variant:ident, $ty:ty) => {
        pub(super) fn $name(&self) -> Option<&$ty> {
            match &self.payload {
                Payload::$variant(value) => Some(value),
                _ => None,
            }
        }

        pub(super) fn $name_mut(&mut self) -> Option<&mut $ty> {
            match &mut self.payload {
                Payload::$variant(value) => Some(value),
                _ => None,
            }
        }
    };
}

impl Node {
    pub(super) fn new(kind: Kind, style: Style, taffy: TaffyId, payload: Payload) -> Self {
        let mut flags = NodeFlags::LAYOUT_DIRTY | NodeFlags::SEMANTICS_DIRTY;
        let scroll = if kind == Kind::Scroll {
            flags.insert(NodeFlags::RELAYOUT_BOUNDARY | NodeFlags::REPAINT_BOUNDARY);
            Some(Box::new(ScrollGeom::fresh(crate::motion::ScrollAxis::Both)))
        } else {
            None
        };
        Self {
            kind,
            role: Role::empty(),
            payload,
            paint: None,
            canvas: None,
            scroll,
            transform: None,
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
            content_x: 0.0,
            content_y: 0.0,
            content_w: 0.0,
            content_h: 0.0,
            flags,
            z_index: 0,
            drag_claim: DragClaim::None,
            recognizers: crate::input::RecognizerSet::default(),
            semantics: None,
        }
    }

    pub(super) fn is_text(&self) -> bool {
        self.kind == Kind::Text
    }

    pub(super) fn is_image(&self) -> bool {
        self.kind == Kind::Image
    }

    pub(super) fn is_scroll(&self) -> bool {
        self.kind == Kind::Scroll
    }

    pub(super) fn is_stack(&self) -> bool {
        self.kind == Kind::Stack
    }

    pub(super) fn is_view(&self) -> bool {
        self.kind == Kind::View
    }

    /// Text and image measure themselves. Every other kind is a box.
    pub(super) fn is_box(&self) -> bool {
        self.is_view() || self.is_stack() || self.is_scroll()
    }

    pub(super) fn is_pointer(&self) -> bool {
        self.role.contains(Role::POINTER)
    }

    pub(super) fn is_author_grid(&self) -> bool {
        self.role.contains(Role::GRID)
    }

    pub(super) fn pushes_opacity(&self) -> bool {
        self.alpha().is_some_and(|alpha| alpha != 255)
    }

    pub(super) fn paints_transform(&self) -> bool {
        self.transform
            .as_ref()
            .is_some_and(|geom| !geom.is_identity())
    }

    pub(super) fn is_row_flex(&self) -> bool {
        self.style.display == Display::Flex
            && matches!(
                self.style.flex_direction,
                FlexDirection::Row | FlexDirection::RowReverse
            )
    }

    pub(super) fn is_column_flex(&self) -> bool {
        self.style.display == Display::Flex
            && matches!(
                self.style.flex_direction,
                FlexDirection::Column | FlexDirection::ColumnReverse
            )
    }

    pub(super) fn is_flex_line(&self) -> bool {
        self.is_row_flex() || self.is_column_flex()
    }

    /// Debug label. Padding, align, and sized are views, so the name follows
    /// the computed flex direction rather than the constructor that made them.
    pub(super) fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Scroll => "scroll",
            Kind::Stack if self.role.contains(Role::CLIP) => "clip",
            Kind::Stack if self.role.contains(Role::TRANSFORM) => "transform",
            Kind::Stack if self.alpha().is_some() => "opacity",
            Kind::Stack => "stack",
            Kind::View if self.is_author_grid() => "grid",
            Kind::View if self.is_pointer() => "pointer",
            Kind::View if self.is_row_flex() => "row",
            Kind::View if self.is_column_flex() => "column",
            Kind::View => "view",
        }
    }

    payload_accessors!(text, text_mut, Text, TextPayload);
    payload_accessors!(image, image_mut, Image, ImagePayload);

    /// True when this node's subtree must composite through its own layer.
    ///
    /// A full-alpha opacity and an identity transform draw straight into the
    /// parent picture, the way a settled `FadeTransition` or a parked page does.
    pub(super) fn composites(&self) -> bool {
        if self.is_scroll() || self.paints_transform() || self.pushes_opacity() {
            return true;
        }
        if self.role.contains(Role::CLIP) {
            return false;
        }
        self.paints_effect()
    }

    /// Blur, backdrop blur, or a blend other than [`crate::scene::BlendMode::SrcOver`].
    pub(super) fn paints_effect(&self) -> bool {
        let live = |sigma: f32| sigma.is_finite() && sigma > 0.0;
        live(self.blur())
            || live(self.backdrop())
            || self.blend() != crate::scene::BlendMode::SrcOver
    }

    /// This node opens layers and owns the runs under it this frame.
    ///
    /// `subtree` is true when a strict descendant opens layers. A clip uses that
    /// to decide whether its bracket is a layer or commands inside the parent run.
    /// Alpha 0 and a full alpha do not open an opacity layer. An identity transform
    /// does not open a transform layer. Effects do, on any kind.
    pub(super) fn renders_layers(&self, subtree: bool) -> bool {
        if self.alpha() == Some(0) {
            return false;
        }
        self.paints_effect() || self.composites() || (self.role.contains(Role::CLIP) && subtree)
    }

    /// Scroll keeps one child picture. Other layers keep runs.
    pub(super) fn caches_picture(&self) -> bool {
        self.kind == Kind::Scroll
    }

    pub(super) fn hit_role(&self) -> Hit {
        if self.is_text() || self.is_image() {
            Hit::Opaque
        } else if self.is_pointer() {
            Hit::Region
        } else {
            Hit::Defer
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

    pub(super) fn corners(&self) -> Option<crate::scene::CornerShape> {
        let paint = self.paint.as_deref()?;
        (paint.corner_radii.is_some() || paint.corner_curve != 2.0).then(|| {
            crate::scene::CornerShape {
                radii: paint.corner_radii.unwrap_or([paint.corner_radius; 4]),
                curve: paint.corner_curve,
            }
        })
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

    pub(super) fn gradient(&self) -> Option<&crate::scene::Gradient> {
        self.paint
            .as_ref()
            .and_then(|paint| paint.gradient.as_ref())
    }

    pub(super) fn box_shadows(&self) -> Arc<[crate::scene::BoxShadow]> {
        self.paint
            .as_ref()
            .map(|paint| Arc::clone(&paint.box_shadows))
            .unwrap_or_else(|| Arc::from([]))
    }

    pub(super) fn shadow(&self) -> Option<crate::scene::Shadow> {
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

    pub(super) fn blend(&self) -> crate::scene::BlendMode {
        self.paint
            .as_ref()
            .map(|paint| paint.blend)
            .unwrap_or(crate::scene::BlendMode::SrcOver)
    }

    pub(super) fn scroll_offset(&self) -> (f32, f32) {
        self.scroll
            .as_ref()
            .map(|geom| (geom.offset_x, geom.offset_y))
            .unwrap_or((0.0, 0.0))
    }

    /// Offset plus rubber-band. Record and hit testing use this pair.
    pub(super) fn scroll_paint(&self) -> (f32, f32) {
        self.scroll
            .as_ref()
            .map(|geom| {
                (
                    geom.offset_x + geom.overscroll_x,
                    geom.offset_y + geom.overscroll_y,
                )
            })
            .unwrap_or((0.0, 0.0))
    }

    pub(super) fn overscroll(&self) -> (f32, f32) {
        self.scroll
            .as_ref()
            .map(|geom| (geom.overscroll_x, geom.overscroll_y))
            .unwrap_or((0.0, 0.0))
    }

    pub(super) fn scroll_geom(&self) -> Option<&ScrollGeom> {
        self.scroll.as_deref()
    }

    pub(super) fn scroll_mut(&mut self) -> Option<&mut ScrollGeom> {
        self.scroll.as_deref_mut()
    }

    pub(super) fn transform(&self) -> TransformGeom {
        self.transform
            .as_deref()
            .copied()
            .unwrap_or_else(TransformGeom::identity)
    }

    pub(super) fn transform_mut(&mut self) -> Option<&mut TransformGeom> {
        self.transform.as_deref_mut()
    }
}

pub(super) struct Slot {
    pub(super) generation: u32,
    pub(super) node: Option<Node>,
}
