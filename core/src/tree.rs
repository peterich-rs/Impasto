use std::cell::Cell;

use crate::{Error, Phase, PointerEvent, SINGLE_LINE_MAX, Shaper, TextSize};
use protocol::{Command, DrawList, TextAlign, TextStyle};
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

/// Generation stops here so `to_raw` stays below 2^53 and a JS number can hold it.
const MAX_GENERATION: u32 = (1 << 21) - 1;

/// A node in one [`Tree`]. The slot index is stable. `generation` changes when
/// the slot is recycled, so a stale id cannot address the node that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    index: u32,
    generation: u32,
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

    fn slot(self) -> usize {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
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
enum Hit {
    /// The box itself is a target when no child takes the point.
    Opaque,
    /// Only children can be hit.
    Defer,
    /// The box is a click target. Children are tested first.
    Region,
}

struct TextNode {
    utf8: String,
    font_size: f32,
    color: [u8; 4],
    /// Paragraph constraint passed to [`Shaper::measure`] for this layout.
    max_width: Cell<f32>,
    style: TextStyle,
}

struct ImageNode {
    source: String,
    width: f32,
    height: f32,
}

struct Node {
    kind: Kind,
    /// Copied at creation. Revive and the setters read this, not the live Taffy style.
    style: Style,
    /// `None` after the subtree's Taffy nodes were released. The scene node stays.
    taffy: Option<TaffyId>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    hit: Hit,
    text: Option<TextNode>,
    image: Option<ImageNode>,
    fill: Option<[u8; 4]>,
    /// Corner radius for [`Kind::Decorated`] and [`Kind::Clip`]. Other kinds keep `0.0`.
    corner_radius: f32,
    /// Layer alpha for [`Kind::Opacity`].
    alpha: Option<u8>,
    /// Paint and hit translation for [`Kind::Scroll`]. Not a layout input.
    scroll: (f32, f32),
    translate_x: f32,
    translate_y: f32,
    rotate_rad: f32,
    /// Uniform scale for [`Kind::Transform`]. `1.0` leaves size unchanged.
    scale: f32,
    /// Tap-to-focus target. Containers start cleared. Text and images do too.
    focusable: bool,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// Distance from the pointer-down point that a move must exceed before a scroll
/// wins the gesture. The same length as Flutter's `kTouchSlop`.
const DRAG_SLOP: f32 = 8.0;

#[derive(Clone, Copy)]
struct Sample {
    x: f32,
    y: f32,
    time_ms: f64,
}

struct DragState {
    scroll: Option<NodeId>,
    locked: bool,
    origin_x: f32,
    origin_y: f32,
    last_x: f32,
    last_y: f32,
    previous_move: Option<Sample>,
    latest_move: Option<Sample>,
}

/// Render tree. `layout` writes positions. `pointer` updates click and drag.
///
/// The shaper is erased here so one tree type serves the test shaper and the
/// host's Impeller shaper. The trait is not `Send`: an Impeller typography
/// context stays on the UI thread, and so does the tree.
pub struct Tree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    taffy: TaffyTree<usize>,
    shaper: Box<dyn Shaper>,
    root: Option<NodeId>,
    pressed: Option<NodeId>,
    focused: Option<NodeId>,
    drag: Option<DragState>,
    dirty: bool,
    laid_out: bool,
    /// Content origin in scene points. Insets shift the root; `(0, 0)` is the surface origin.
    origin: (f32, f32),
    viewport: Option<(f32, f32)>,
}

impl Tree {
    pub fn new(shaper: impl Shaper + 'static) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            taffy: TaffyTree::new(),
            shaper: Box::new(shaper),
            root: None,
            pressed: None,
            focused: None,
            drag: None,
            dirty: false,
            laid_out: false,
            origin: (0.0, 0.0),
            viewport: None,
        }
    }

    /// Content box for the next `layout`, in scene points.
    ///
    /// `(x, y)` is where the root is placed. `width` and `height` are the space
    /// Taffy may use. Unset size keeps `MAX_CONTENT`. Does not drop the previous
    /// geometry. Hit testing uses that geometry until `layout` runs again.
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32) {
        self.origin = (finite_coord(x), finite_coord(y));
        self.viewport = Some((finite_size(width), finite_size(height)));
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn row(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(Kind::Row, Hit::Defer, flex_style(FlexDirection::Row, gap))
    }

    pub fn column(&mut self, gap: f32) -> Result<NodeId, Error> {
        self.container(
            Kind::Column,
            Hit::Defer,
            flex_style(FlexDirection::Column, gap),
        )
    }

    pub fn grid(&mut self, columns: u16) -> Result<NodeId, Error> {
        let tracks = (0..columns).map(|_| min_content()).collect();
        self.container(
            Kind::Grid,
            Hit::Defer,
            Style {
                display: Display::Grid,
                grid_template_columns: tracks,
                align_items: Some(AlignItems::Start),
                justify_items: Some(AlignItems::Start),
                ..Style::default()
            },
        )
    }

    pub fn padding(&mut self, edges: f32) -> Result<NodeId, Error> {
        let edge = length(edges);
        self.container(
            Kind::Padding,
            Hit::Defer,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Start),
                padding: taffy::Rect {
                    left: edge,
                    right: edge,
                    top: edge,
                    bottom: edge,
                },
                ..Style::default()
            },
        )
    }

    pub fn align(&mut self, alignment: Alignment) -> Result<NodeId, Error> {
        let (cross, main) = align_axes(alignment);
        self.container(
            Kind::Align,
            Hit::Defer,
            Style {
                display: Display::Flex,
                align_items: Some(cross),
                justify_content: Some(main),
                ..Style::default()
            },
        )
    }

    pub fn sized(&mut self, width: f32, height: f32) -> Result<NodeId, Error> {
        self.container(
            Kind::Sized,
            Hit::Defer,
            Style {
                size: Size {
                    width: length(width),
                    height: length(height),
                },
                ..Style::default()
            },
        )
    }

    pub fn stack(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Stack, Hit::Defer, stack_style())
    }

    /// Clips descendants to this border box. A radius of zero is a rectangle.
    ///
    /// Hit testing uses that rectangle. The rounded outline does not change hits.
    pub fn clip(&mut self, corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(Kind::Clip, Hit::Defer, stack_style())?;
        self.node_mut(id)?.corner_radius = corner_radius;
        Ok(id)
    }

    /// Composites descendants as one layer. `alpha` is 0–255.
    pub fn opacity(&mut self, alpha: u8) -> Result<NodeId, Error> {
        let id = self.container(Kind::Opacity, Hit::Defer, stack_style())?;
        self.node_mut(id)?.alpha = Some(alpha);
        Ok(id)
    }

    /// A viewport. Children lay out at their full size and may overflow the box.
    ///
    /// The border box is the viewport. `set_scroll_offset` moves paint and hit
    /// testing only, so a scroll frame does not invalidate Taffy's layout cache.
    pub fn scroll(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Scroll, Hit::Defer, scroll_style())
    }

    /// A paint transform around the current origin. Layout and hit testing ignore it.
    ///
    /// The initial transform is identity: no translation, no rotation, scale 1.
    pub fn transform(&mut self) -> Result<NodeId, Error> {
        self.container(Kind::Transform, Hit::Defer, stack_style())
    }

    pub fn text(
        &mut self,
        utf8: impl Into<String>,
        font_size: f32,
        color: [u8; 4],
    ) -> Result<NodeId, Error> {
        let utf8 = utf8.into();
        let style = Style::default();
        let (index, generation) = self.reserve();
        let taffy = match self
            .taffy
            .new_leaf_with_context(style.clone(), index as usize)
        {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
        Ok(self.occupy(
            index,
            generation,
            Node {
                kind: Kind::Text,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit: Hit::Opaque,
                text: Some(TextNode {
                    utf8,
                    font_size,
                    color,
                    max_width: Cell::new(0.0),
                    style: TextStyle::default(),
                }),
                image: None,
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
    }

    pub fn image(
        &mut self,
        source: impl Into<String>,
        width: f32,
        height: f32,
    ) -> Result<NodeId, Error> {
        let style = Style::default();
        let (index, generation) = self.reserve();
        let taffy = match self
            .taffy
            .new_leaf_with_context(style.clone(), index as usize)
        {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
        Ok(self.occupy(
            index,
            generation,
            Node {
                kind: Kind::Image,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit: Hit::Opaque,
                text: None,
                image: Some(ImageNode {
                    source: source.into(),
                    width,
                    height,
                }),
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
    }

    /// Paints its border box and sizes itself to its children.
    ///
    /// `corner_radius` is in scene points. Zero paints a sharp rectangle and does not
    /// change hit testing.
    pub fn decorated(&mut self, color: [u8; 4], corner_radius: f32) -> Result<NodeId, Error> {
        let id = self.container(
            Kind::Decorated,
            Hit::Defer,
            flex_style(FlexDirection::Column, 0.0),
        )?;
        let node = self.node_mut(id)?;
        node.fill = Some(color);
        node.corner_radius = corner_radius;
        Ok(id)
    }

    pub fn pointer_region(&mut self) -> Result<NodeId, Error> {
        self.container(
            Kind::PointerRegion,
            Hit::Region,
            Style {
                display: Display::Flex,
                align_items: Some(AlignItems::Start),
                justify_content: Some(JustifyContent::Start),
                ..Style::default()
            },
        )
    }

    pub fn add_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        self.insert(parent, child, None)
    }

    /// Places `child` under `parent`, before `before` when that anchor is set.
    ///
    /// A child that already has a parent moves. A child whose Taffy nodes were
    /// released is rebuilt from the stored style and child list.
    pub fn insert(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), Error> {
        if parent == child || self.is_under(parent, child)? {
            return Err(Error::NotAChild);
        }
        if !can_parent(self.node(parent)?.kind) {
            return Err(Error::NotAParent);
        }
        let _ = self.node(child)?;
        if let Some(anchor) = before {
            if anchor == child {
                return if self.node(child)?.parent == Some(parent) {
                    Ok(())
                } else {
                    Err(Error::NotAChild)
                };
            }
            if self.node(anchor)?.parent != Some(parent) {
                return Err(Error::NotAChild);
            }
        }

        if self.node(parent)?.taffy.is_none() {
            self.revive(parent)?;
        }
        if self.node(child)?.taffy.is_none() {
            self.revive(child)?;
        }
        if self.node(child)?.parent.is_some() {
            self.unlink(child)?;
        } else if self.root == Some(child) {
            self.root = None;
        }

        self.pin_stack_child(parent, child)?;
        let index = match before {
            Some(anchor) => self
                .node(parent)?
                .children
                .iter()
                .position(|id| *id == anchor)
                .ok_or(Error::NotAChild)?,
            None => self.node(parent)?.children.len(),
        };
        let parent_taffy = self.node(parent)?.taffy.ok_or(Error::UnknownNode)?;
        let child_taffy = self.node(child)?.taffy.ok_or(Error::UnknownNode)?;
        self.taffy
            .insert_child_at_index(parent_taffy, index, child_taffy)?;
        self.node_mut(parent)?.children.insert(index, child);
        self.node_mut(child)?.parent = Some(parent);
        self.promote(parent);
        self.mark_structure();
        Ok(())
    }

    /// Detaches `child` from `parent` and releases the subtree's Taffy nodes.
    ///
    /// The scene nodes stay, with the same ids, so the subtree can be inserted
    /// again. Handlers on a detached region are never hit. A focused node in
    /// the subtree produces [`Gesture::Blur`]. Inserting elsewhere does not.
    pub fn remove(&mut self, parent: NodeId, child: NodeId) -> Result<Vec<Gesture>, Error> {
        if self.node(child)?.parent != Some(parent) {
            return Err(Error::NotAChild);
        }
        self.node_mut(parent)?.children.retain(|id| *id != child);
        self.node_mut(child)?.parent = None;
        self.release_taffy(child)?;
        let gestures = self.blur_contained(child);
        self.mark_structure();
        Ok(gestures)
    }

    /// Retires `id` and every descendant. The caller guarantees `id` has no parent.
    ///
    /// `remove` leaves the slots so the subtree can be inserted again. `discard`
    /// is the step that makes those ids unknown. The returned ids are the ones
    /// that were retired, children before their parent. A focused node in the
    /// subtree also produces [`Gesture::Blur`].
    pub fn discard(&mut self, id: NodeId) -> Result<Discarded, Error> {
        if self.node(id)?.parent.is_some() {
            return Err(Error::NotAChild);
        }
        let gestures = self.blur_contained(id);
        if self.node(id)?.taffy.is_some() {
            self.release_taffy(id)?;
        }
        let mut ids = Vec::new();
        self.retire_postorder(id, &mut ids)?;
        self.mark_structure();
        Ok(Discarded { ids, gestures })
    }

    fn retire_postorder(&mut self, id: NodeId, retired: &mut Vec<NodeId>) -> Result<(), Error> {
        let children = self.node(id)?.children.clone();
        for child in children {
            self.retire_postorder(child, retired)?;
        }
        retired.push(id);
        self.retire(id.index);
        Ok(())
    }

    pub fn parent(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.parent)
    }

    pub fn first_child(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        Ok(self.node(id)?.children.first().copied())
    }

    pub fn next_sibling(&self, id: NodeId) -> Result<Option<NodeId>, Error> {
        let Some(parent) = self.node(id)?.parent else {
            return Ok(None);
        };
        let children = &self.node(parent)?.children;
        let Some(index) = children.iter().position(|child| *child == id) else {
            return Ok(None);
        };
        Ok(children.get(index + 1).copied())
    }

    pub fn is_text(&self, id: NodeId) -> Result<bool, Error> {
        Ok(self.node(id)?.kind == Kind::Text)
    }

    /// Replaces the text. The previous geometry stays valid for hit testing
    /// until the next `layout`.
    pub fn set_text(&mut self, id: NodeId, utf8: impl Into<String>) -> Result<(), Error> {
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            text.utf8 = utf8.into();
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    pub fn set_font_size(&mut self, id: NodeId, font_size: f32) -> Result<(), Error> {
        finite_number(font_size)?;
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            text.font_size = font_size;
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    /// Foreground for text, fill for a decorated box.
    pub fn set_color(&mut self, id: NodeId, color: [u8; 4]) -> Result<(), Error> {
        let kind = self.node(id)?.kind;
        match kind {
            Kind::Text => {
                let taffy_id = {
                    let node = self.node_mut(id)?;
                    let Some(text) = node.text.as_mut() else {
                        return Err(Error::NotText);
                    };
                    text.color = color;
                    node.taffy
                };
                self.touch_measure(taffy_id)
            }
            Kind::Decorated => {
                self.node_mut(id)?.fill = Some(color);
                self.dirty = true;
                Ok(())
            }
            _ => Err(Error::WrongKind),
        }
    }

    pub fn set_gap(&mut self, id: NodeId, gap: f32) -> Result<(), Error> {
        finite_number(gap)?;
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.gap = Size {
            width: length(gap),
            height: length(gap),
        };
        self.write_style(id, style)
    }

    /// Sets flex grow on this node. The value must be zero or positive.
    ///
    /// Marks the tree dirty and keeps the previous geometry until the next `layout`.
    /// Free space exists when the flex parent has a definite main-axis size.
    pub fn set_grow(&mut self, id: NodeId, grow: f32) -> Result<(), Error> {
        if !(grow >= 0.0) {
            return Err(Error::NegativeGrow);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_grow = grow;
        self.write_style(id, style)
    }

    pub fn set_padding(&mut self, id: NodeId, edges: f32) -> Result<(), Error> {
        finite_number(edges)?;
        if self.node(id)?.kind != Kind::Padding {
            return Err(Error::WrongKind);
        }
        let edge = length(edges);
        let mut style = self.node(id)?.style.clone();
        style.padding = taffy::Rect {
            left: edge,
            right: edge,
            top: edge,
            bottom: edge,
        };
        self.write_style(id, style)
    }

    /// Corner radius of a decorated box or a clip, in scene points.
    pub fn set_radius(&mut self, id: NodeId, radius: f32) -> Result<(), Error> {
        let radius = finite_positive(radius)?;
        let node = self.node_mut(id)?;
        if !matches!(node.kind, Kind::Decorated | Kind::Clip) {
            return Err(Error::WrongKind);
        }
        node.corner_radius = radius;
        self.dirty = true;
        Ok(())
    }

    /// Layer alpha, 0–255. Geometry stays until the next layout.
    pub fn set_alpha(&mut self, id: NodeId, alpha: u8) -> Result<(), Error> {
        let node = self.node_mut(id)?;
        if node.kind != Kind::Opacity {
            return Err(Error::WrongKind);
        }
        node.alpha = Some(alpha);
        self.dirty = true;
        Ok(())
    }

    pub fn set_align(&mut self, id: NodeId, alignment: Alignment) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Align {
            return Err(Error::WrongKind);
        }
        let (cross, main) = align_axes(alignment);
        let mut style = self.node(id)?.style.clone();
        style.align_items = Some(cross);
        style.justify_content = Some(main);
        self.write_style(id, style)
    }

    /// Main-axis alignment of a row or column.
    pub fn set_justify(&mut self, id: NodeId, justify: Justify) -> Result<(), Error> {
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.justify_content = Some(justify_content(justify));
        self.write_style(id, style)
    }

    /// `true` lets a row or column wrap onto another line.
    pub fn set_wrap(&mut self, id: NodeId, wrap: bool) -> Result<(), Error> {
        if !matches!(self.node(id)?.kind, Kind::Row | Kind::Column) {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_wrap = if wrap {
            FlexWrap::Wrap
        } else {
            FlexWrap::NoWrap
        };
        self.write_style(id, style)
    }

    /// Width and height become 100% of the parent, or of the viewport when this is the root.
    ///
    /// The minimum size is zero so a flex parent can shrink this node.
    pub fn set_fill(&mut self, id: NodeId) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: percent(1.0),
            height: percent(1.0),
        };
        style.min_size = Size {
            width: length(0.0),
            height: length(0.0),
        };
        self.write_style(id, style)
    }

    pub fn set_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        if self.node(id)?.kind != Kind::Sized {
            return Err(Error::WrongKind);
        }
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)
    }

    pub fn set_image(
        &mut self,
        id: NodeId,
        source: &str,
        width: f32,
        height: f32,
    ) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(image) = node.image.as_mut() else {
                return Err(Error::WrongKind);
            };
            image.source = source.to_owned();
            image.width = width;
            image.height = height;
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    pub fn set_max_lines(&mut self, id: NodeId, max_lines: Option<u32>) -> Result<(), Error> {
        if matches!(max_lines, Some(0)) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style.max_lines = max_lines)
    }

    pub fn set_ellipsis(&mut self, id: NodeId, ellipsis: Option<&str>) -> Result<(), Error> {
        let ellipsis = match ellipsis {
            Some(text) if !text.is_empty() => Some(text.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| text.style.ellipsis = ellipsis)
    }

    pub fn set_text_align(&mut self, id: NodeId, align: TextAlign) -> Result<(), Error> {
        self.edit_text(id, |text| text.style.align = align)
    }

    pub fn set_font_weight(&mut self, id: NodeId, weight: u16) -> Result<(), Error> {
        if !matches!(weight, 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900) {
            return Err(Error::BadNumber);
        }
        self.edit_text(id, |text| text.style.weight = weight)
    }

    pub fn set_text_direction(&mut self, id: NodeId, rtl: bool) -> Result<(), Error> {
        self.edit_text(id, |text| text.style.direction_rtl = rtl)
    }

    /// `0.0` uses the font's own line height.
    pub fn set_line_height(&mut self, id: NodeId, height: f32) -> Result<(), Error> {
        let height = finite_positive(height)?;
        self.edit_text(id, |text| text.style.height = height)
    }

    /// Scroll paint offset in scene points. Marks the tree dirty and leaves Taffy alone.
    ///
    /// The next `layout` still runs because the tree is dirty, but unchanged styles
    /// keep the cached boxes. `record` and hit testing apply the offset.
    pub fn set_scroll_offset(&mut self, id: NodeId, dx: f32, dy: f32) -> Result<(), Error> {
        finite_number(dx)?;
        finite_number(dy)?;
        let node = self.node_mut(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        node.scroll = (dx, dy);
        self.dirty = true;
        Ok(())
    }

    pub fn scroll_offset(&self, id: NodeId) -> Result<(f32, f32), Error> {
        let node = self.node(id)?;
        if node.kind != Kind::Scroll {
            return Err(Error::WrongKind);
        }
        Ok(node.scroll)
    }

    /// Paint transform. Scale is uniform. Rotation is clockwise radians.
    ///
    /// Does not mark Taffy dirty. Hit testing keeps using the untransformed box.
    pub fn set_transform(
        &mut self,
        id: NodeId,
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale: f32,
    ) -> Result<(), Error> {
        finite_number(translate_x)?;
        finite_number(translate_y)?;
        finite_number(rotate_rad)?;
        finite_number(scale)?;
        let node = self.node_mut(id)?;
        if node.kind != Kind::Transform {
            return Err(Error::WrongKind);
        }
        node.translate_x = translate_x;
        node.translate_y = translate_y;
        node.rotate_rad = rotate_rad;
        node.scale = scale;
        self.dirty = true;
        Ok(())
    }

    /// `None` or `""` uses the family registered on the player.
    pub fn set_family(&mut self, id: NodeId, family: Option<&str>) -> Result<(), Error> {
        let family = match family {
            Some(name) if !name.is_empty() => Some(name.to_owned()),
            _ => None,
        };
        self.edit_text(id, |text| text.style.family = family)
    }

    fn edit_text(&mut self, id: NodeId, edit: impl FnOnce(&mut TextNode)) -> Result<(), Error> {
        let taffy_id = {
            let node = self.node_mut(id)?;
            let Some(text) = node.text.as_mut() else {
                return Err(Error::NotText);
            };
            edit(text);
            node.taffy
        };
        self.touch_measure(taffy_id)
    }

    pub fn layout(&mut self) -> Result<(), Error> {
        if !self.dirty && self.laid_out {
            return Ok(());
        }
        let root = self.root.ok_or(Error::NoRoot)?;
        let root_taffy = self.node(root)?.taffy.ok_or(Error::NoRoot)?;
        let available = match self.viewport {
            Some((width, height)) => Size {
                width: AvailableSpace::Definite(width),
                height: AvailableSpace::Definite(height),
            },
            None => Size::MAX_CONTENT,
        };
        let slots = &self.slots;
        let shaper = self.shaper.as_ref();
        self.taffy.compute_layout_with_measure(
            root_taffy,
            available,
            |known, available, _id, context, _style| {
                measure_leaf(slots, shaper, known, available, context)
            },
        )?;
        self.place(root, self.origin.0, self.origin.1)?;
        self.dirty = false;
        self.laid_out = true;
        Ok(())
    }

    /// Same shaper `layout` used for this tree. `max_width` and `color` match
    /// [`Shaper::measure`], so a cursor measurement can use the paragraph the
    /// text node was laid out with.
    pub fn measure_text(
        &self,
        utf8: &str,
        font_size: f32,
        max_width: f32,
        color: [u8; 4],
        style: &TextStyle,
    ) -> TextSize {
        self.shaper
            .measure(utf8, font_size, max_width, color, style)
    }

    pub fn rect(&self, id: NodeId) -> Result<Rect, Error> {
        let node = self.node(id)?;
        Ok(Rect {
            x: node.x,
            y: node.y,
            w: node.w,
            h: node.h,
        })
    }

    pub fn record(&self) -> Result<DrawList, Error> {
        let mut commands = Vec::new();
        self.record_node(self.root.ok_or(Error::NoRoot)?, &mut commands, (0.0, 0.0));
        Ok(DrawList { commands })
    }

    /// Marks `id` as a tap-to-focus target. Any node may carry the flag.
    /// Clearing it on the focused node blurs.
    pub fn set_focusable(&mut self, id: NodeId, focusable: bool) -> Result<Vec<Gesture>, Error> {
        self.node_mut(id)?.focusable = focusable;
        if !focusable && self.focused == Some(id) {
            Ok(self.move_focus(None))
        } else {
            Ok(Vec::new())
        }
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Moves focus to `id` when it is marked focusable. The returned transition
    /// is empty when `id` is already focused.
    pub fn focus(&mut self, id: NodeId) -> Result<Vec<Gesture>, Error> {
        if !self.node(id)?.focusable {
            return Err(Error::NotFocusable);
        }
        Ok(self.move_focus(Some(id)))
    }

    pub fn blur(&mut self) -> Vec<Gesture> {
        self.move_focus(None)
    }

    /// Hit-tests the previous layout and updates the press, drag, and focus gestures.
    ///
    /// A hit on the way down is [`Gesture::Press`]. A move that stays within
    /// [`DRAG_SLOP`] keeps that press. Past the slop, the nearest [`Kind::Scroll`]
    /// ancestor locks the pointer: the press is released and later moves are
    /// [`Gesture::DragBy`]. A lift inside the pressed region, with no lock, is
    /// [`Gesture::Release`] then [`Gesture::Click`], and that click is what moves
    /// focus. A locked drag does not. Does not lay out.
    pub fn pointer(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if !self.laid_out {
            return Vec::new();
        }
        match event.phase {
            Phase::Down => self.pointer_down(event),
            Phase::Move => self.pointer_move(event),
            Phase::Up => self.pointer_up(event),
            Phase::Cancel => self.pointer_cancel(),
        }
    }

    fn pointer_down(&mut self, event: PointerEvent) -> Vec<Gesture> {
        self.drag = None;
        self.pressed = None;
        let hit = self.hit_root(event.x, event.y);
        let pressed = hit.and_then(|id| self.region_of(id));
        let scroll = hit.and_then(|id| self.drag_of(id));
        self.pressed = pressed;
        if pressed.is_some() || scroll.is_some() {
            self.drag = Some(DragState {
                scroll,
                locked: false,
                origin_x: event.x,
                origin_y: event.y,
                last_x: event.x,
                last_y: event.y,
                previous_move: None,
                latest_move: None,
            });
        }
        pressed.into_iter().map(Gesture::Press).collect()
    }

    fn pointer_move(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if self.drag.as_ref().is_some_and(|drag| drag.locked) {
            return self.drag_by(event);
        }
        let Some(drag) = self.drag.as_ref() else {
            return Vec::new();
        };
        let past = past_slop(drag.origin_x, drag.origin_y, event.x, event.y);
        let scroll = drag.scroll.filter(|_| past);
        if let Some(scroll) = scroll {
            return self.lock_drag(event, scroll);
        }
        self.move_pressed(event)
    }

    fn drag_by(&mut self, event: PointerEvent) -> Vec<Gesture> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let Some(scroll) = drag.scroll else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        vec![Gesture::DragBy(scroll, dx, dy)]
    }

    fn lock_drag(&mut self, event: PointerEvent, scroll: NodeId) -> Vec<Gesture> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let (dx, dy) = note_move(drag, event);
        drag.locked = true;
        let mut gestures = Vec::new();
        if let Some(target) = self.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        gestures.push(Gesture::DragBegin(scroll));
        gestures.push(Gesture::DragBy(scroll, dx, dy));
        gestures
    }

    fn move_pressed(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if let Some(drag) = self.drag.as_mut() {
            note_move(drag, event);
        }
        let Some(target) = self.pressed else {
            return Vec::new();
        };
        let still = self
            .hit_root(event.x, event.y)
            .is_some_and(|hit| self.contains(target, hit));
        if still {
            Vec::new()
        } else {
            self.pressed = None;
            self.drag = None;
            vec![Gesture::Release(target)]
        }
    }

    fn pointer_up(&mut self, event: PointerEvent) -> Vec<Gesture> {
        if self.drag.as_ref().is_some_and(|drag| drag.locked) {
            let scroll = self.drag.as_ref().and_then(|drag| drag.scroll);
            let speed = self.drag.as_ref().map(drag_velocity).unwrap_or((0.0, 0.0));
            self.drag = None;
            self.pressed = None;
            return scroll
                .map(|id| vec![Gesture::DragEnd(id, speed.0, speed.1)])
                .unwrap_or_default();
        }
        self.drag = None;
        let Some(target) = self.pressed.take() else {
            return Vec::new();
        };
        let hit = self.hit_root(event.x, event.y);
        let inside = hit.is_some_and(|hit| self.contains(target, hit));
        if !inside {
            return vec![Gesture::Release(target)];
        }
        let mut gestures = vec![Gesture::Release(target), Gesture::Click(target)];
        let next = hit.and_then(|id| self.focusable_of(id));
        gestures.extend(self.move_focus(next));
        gestures
    }

    fn pointer_cancel(&mut self) -> Vec<Gesture> {
        let mut gestures = Vec::new();
        if let Some(target) = self.pressed.take() {
            gestures.push(Gesture::Release(target));
        }
        if self.drag.as_ref().is_some_and(|drag| drag.locked)
            && let Some(scroll) = self.drag.as_ref().and_then(|drag| drag.scroll)
        {
            gestures.push(Gesture::DragEnd(scroll, 0.0, 0.0));
        }
        self.drag = None;
        gestures
    }

    fn container(&mut self, kind: Kind, hit: Hit, style: Style) -> Result<NodeId, Error> {
        let (index, generation) = self.reserve();
        let taffy = match self.taffy.new_leaf(style.clone()) {
            Ok(id) => id,
            Err(error) => {
                self.retire(index);
                return Err(error.into());
            }
        };
        Ok(self.occupy(
            index,
            generation,
            Node {
                kind,
                style,
                taffy: Some(taffy),
                parent: None,
                children: Vec::new(),
                hit,
                text: None,
                image: None,
                fill: None,
                corner_radius: 0.0,
                alpha: None,
                scroll: (0.0, 0.0),
                translate_x: 0.0,
                translate_y: 0.0,
                rotate_rad: 0.0,
                scale: 1.0,
                focusable: false,
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
        ))
    }

    /// The first node becomes the root so a one-node tree can lay out. Moving
    /// that node under a parent promotes the new top.
    fn occupy(&mut self, index: u32, generation: u32, node: Node) -> NodeId {
        self.slots[index as usize].node = Some(node);
        let id = NodeId { index, generation };
        if self.root.is_none() {
            self.root = Some(id);
        }
        id
    }

    fn reserve(&mut self) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let generation = self.slots[index as usize].generation;
            (index, generation)
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 1,
                node: None,
            });
            (index, 1)
        }
    }

    /// Drops whatever occupies `index` and recycles it until the generation cap.
    fn retire(&mut self, index: u32) {
        if self.root.is_some_and(|root| root.index == index) {
            self.root = None;
        }
        if self.pressed.is_some_and(|pressed| pressed.index == index) {
            self.pressed = None;
        }
        if self.focused.is_some_and(|focused| focused.index == index) {
            self.focused = None;
        }
        if self
            .drag
            .as_ref()
            .is_some_and(|drag| drag.scroll.is_some_and(|scroll| scroll.index == index))
        {
            self.drag = None;
        }
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return;
        };
        slot.node = None;
        if slot.generation >= MAX_GENERATION {
            return;
        }
        slot.generation += 1;
        self.free.push(index);
    }

    fn node(&self, id: NodeId) -> Result<&Node, Error> {
        let slot = self.slots.get(id.slot()).ok_or(Error::UnknownNode)?;
        if slot.generation != id.generation {
            return Err(Error::UnknownNode);
        }
        slot.node.as_ref().ok_or(Error::UnknownNode)
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, Error> {
        let slot = self.slots.get_mut(id.slot()).ok_or(Error::UnknownNode)?;
        if slot.generation != id.generation {
            return Err(Error::UnknownNode);
        }
        slot.node.as_mut().ok_or(Error::UnknownNode)
    }

    fn get(&self, id: NodeId) -> Option<&Node> {
        self.node(id).ok()
    }

    fn is_under(&self, mut node: NodeId, ancestor: NodeId) -> Result<bool, Error> {
        let mut guard = self.slots.len() + 1;
        loop {
            if node == ancestor {
                return Ok(true);
            }
            if guard == 0 {
                return Err(Error::NotAChild);
            }
            guard -= 1;
            let Some(parent) = self.node(node)?.parent else {
                return Ok(false);
            };
            node = parent;
        }
    }

    fn unlink(&mut self, child: NodeId) -> Result<(), Error> {
        let Some(parent) = self.node(child)?.parent else {
            return Ok(());
        };
        self.node_mut(parent)?.children.retain(|id| *id != child);
        let parent_taffy = self.node(parent)?.taffy;
        let child_taffy = self.node(child)?.taffy;
        if let (Some(parent_taffy), Some(child_taffy)) = (parent_taffy, child_taffy) {
            self.taffy.remove_child(parent_taffy, child_taffy)?;
        }
        self.node_mut(child)?.parent = None;
        Ok(())
    }

    fn pin_stack_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), Error> {
        if !pins_children(self.node(parent)?.kind) {
            return Ok(());
        }
        let mut style = self.node(child)?.style.clone();
        style.grid_row = line(1);
        style.grid_column = line(1);
        self.write_style(child, style)
    }

    fn write_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy;
        self.node_mut(id)?.style = style.clone();
        if let Some(taffy_id) = taffy_id {
            self.taffy.set_style(taffy_id, style)?;
        }
        self.dirty = true;
        Ok(())
    }

    fn touch_measure(&mut self, taffy_id: Option<TaffyId>) -> Result<(), Error> {
        if let Some(taffy_id) = taffy_id {
            self.taffy.mark_dirty(taffy_id)?;
        }
        self.dirty = true;
        Ok(())
    }

    fn revive(&mut self, id: NodeId) -> Result<(), Error> {
        if self.node(id)?.taffy.is_some() {
            return Ok(());
        }
        let children = self.node(id)?.children.clone();
        for child in &children {
            self.revive(*child)?;
        }
        let style = self.node(id)?.style.clone();
        let kind = self.node(id)?.kind;
        let index = id.index as usize;
        let taffy_id = if matches!(kind, Kind::Text | Kind::Image) {
            self.taffy.new_leaf_with_context(style, index)?
        } else {
            self.taffy.new_leaf(style)?
        };
        if !children.is_empty() {
            let mut child_ids = Vec::with_capacity(children.len());
            for child in &children {
                child_ids.push(self.node(*child)?.taffy.ok_or(Error::UnknownNode)?);
            }
            self.taffy.set_children(taffy_id, &child_ids)?;
        }
        self.node_mut(id)?.taffy = Some(taffy_id);
        Ok(())
    }

    fn release_taffy(&mut self, id: NodeId) -> Result<(), Error> {
        let children = self.node(id)?.children.clone();
        for child in children {
            self.release_taffy(child)?;
        }
        let taffy_id = self.node_mut(id)?.taffy.take();
        if let Some(taffy_id) = taffy_id {
            self.taffy.remove(taffy_id)?;
        }
        Ok(())
    }

    fn promote(&mut self, parent: NodeId) {
        let top = self.top(parent);
        match self.root {
            None => self.root = Some(top),
            Some(root) => {
                let rooted = self
                    .node(root)
                    .ok()
                    .is_some_and(|node| node.parent.is_some());
                if rooted {
                    self.root = Some(top);
                }
            }
        }
    }

    fn top(&self, mut id: NodeId) -> NodeId {
        while let Some(parent) = self.get(id).and_then(|node| node.parent) {
            id = parent;
        }
        id
    }

    fn mark_structure(&mut self) {
        self.dirty = true;
        self.laid_out = false;
        self.pressed = None;
        self.drag = None;
    }

    fn place(&mut self, id: NodeId, origin_x: f32, origin_y: f32) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy.ok_or(Error::UnknownNode)?;
        let (local_x, local_y, w, h) = {
            let layout = self.taffy.layout(taffy_id)?;
            (
                layout.location.x,
                layout.location.y,
                layout.size.width,
                layout.size.height,
            )
        };
        let x = origin_x + local_x;
        let y = origin_y + local_y;
        let node = self.node_mut(id)?;
        node.x = x;
        node.y = y;
        node.w = w;
        node.h = h;
        let children = node.children.clone();
        for child in children {
            self.place(child, x, y)?;
        }
        Ok(())
    }

    fn record_node(&self, id: NodeId, commands: &mut Vec<Command>, offset: (f32, f32)) {
        let Some(node) = self.get(id) else {
            return;
        };
        let x = node.x - offset.0;
        let y = node.y - offset.1;
        if let Some(color) = node.fill
            && node.w > 0.0
            && node.h > 0.0
        {
            commands.push(Command::Rect {
                x,
                y,
                w: node.w,
                h: node.h,
                corner_radius: node.corner_radius,
                color,
            });
        }
        if let Some(text) = &node.text
            && !text.utf8.is_empty()
        {
            commands.push(Command::Text {
                x,
                y,
                max_width: text.max_width.get(),
                font_size: text.font_size,
                color: text.color,
                utf8: text.utf8.clone(),
                style: text.style.clone(),
            });
        }
        if let Some(image) = &node.image {
            commands.push(Command::Image {
                x,
                y,
                w: node.w,
                h: node.h,
                source: image.source.clone(),
            });
        }
        let kind = node.kind;
        let (w, h) = (node.w, node.h);
        let corner_radius = node.corner_radius;
        let alpha = node.alpha;
        let scroll = node.scroll;
        let transform = (
            node.translate_x,
            node.translate_y,
            node.rotate_rad,
            node.scale,
        );
        let child_offset = if kind == Kind::Scroll {
            (offset.0 + scroll.0, offset.1 + scroll.1)
        } else {
            offset
        };
        match kind {
            Kind::Clip => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius,
            }),
            Kind::Opacity => commands.push(Command::PushOpacity {
                x,
                y,
                w,
                h,
                alpha: alpha.unwrap_or(255),
            }),
            Kind::Scroll => commands.push(Command::PushClip {
                x,
                y,
                w,
                h,
                corner_radius: 0.0,
            }),
            Kind::Transform => commands.push(Command::PushTransform {
                translate_x: transform.0,
                translate_y: transform.1,
                rotate_rad: transform.2,
                scale: transform.3,
            }),
            _ => {}
        }
        for child in &node.children {
            self.record_node(*child, commands, child_offset);
        }
        if matches!(
            kind,
            Kind::Clip | Kind::Opacity | Kind::Scroll | Kind::Transform
        ) {
            commands.push(Command::Pop);
        }
    }

    fn hit_root(&self, x: f32, y: f32) -> Option<NodeId> {
        self.root.and_then(|root| self.hit(root, x, y))
    }

    fn hit(&self, id: NodeId, x: f32, y: f32) -> Option<NodeId> {
        let node = self.get(id)?;
        if node.w == 0.0 || node.h == 0.0 {
            return None;
        }
        if x < node.x || y < node.y || x >= node.x + node.w || y >= node.y + node.h {
            return None;
        }
        let (child_x, child_y) = if node.kind == Kind::Scroll {
            (x + node.scroll.0, y + node.scroll.1)
        } else {
            (x, y)
        };
        for child in node.children.iter().rev() {
            if let Some(hit) = self.hit(*child, child_x, child_y) {
                return Some(hit);
            }
        }
        match node.hit {
            Hit::Opaque | Hit::Region => Some(id),
            Hit::Defer => None,
        }
    }

    fn drag_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.kind == Kind::Scroll {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    fn focusable_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.focusable {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    fn move_focus(&mut self, next: Option<NodeId>) -> Vec<Gesture> {
        if self.focused == next {
            return Vec::new();
        }
        let mut gestures = Vec::new();
        if let Some(old) = self.focused.take() {
            gestures.push(Gesture::Blur(old));
        }
        if let Some(id) = next {
            self.focused = Some(id);
            gestures.push(Gesture::Focus(id));
        }
        gestures
    }

    fn blur_contained(&mut self, root: NodeId) -> Vec<Gesture> {
        let Some(focused) = self.focused else {
            return Vec::new();
        };
        match self.is_under(focused, root) {
            Ok(true) => self.move_focus(None),
            _ => Vec::new(),
        }
    }

    fn region_of(&self, mut id: NodeId) -> Option<NodeId> {
        loop {
            let node = self.get(id)?;
            if node.hit == Hit::Region {
                return Some(id);
            }
            id = node.parent?;
        }
    }

    fn contains(&self, target: NodeId, mut hit: NodeId) -> bool {
        loop {
            if hit == target {
                return true;
            }
            let Some(node) = self.get(hit) else {
                return false;
            };
            let Some(parent) = node.parent else {
                return false;
            };
            hit = parent;
        }
    }
}

fn finite_number(value: f32) -> Result<(), Error> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(Error::BadNumber)
    }
}

fn finite_positive(value: f32) -> Result<f32, Error> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(Error::BadNumber)
    }
}

fn align_axes(alignment: Alignment) -> (AlignItems, JustifyContent) {
    match alignment {
        Alignment::Start => (AlignItems::Start, JustifyContent::Start),
        Alignment::Center => (AlignItems::Center, JustifyContent::Center),
        Alignment::End => (AlignItems::End, JustifyContent::End),
    }
}

fn finite_coord(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_size(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn can_parent(kind: Kind) -> bool {
    !matches!(kind, Kind::Text | Kind::Image)
}

fn pins_children(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Stack | Kind::Clip | Kind::Opacity | Kind::Scroll | Kind::Transform
    )
}

fn past_slop(origin_x: f32, origin_y: f32, x: f32, y: f32) -> bool {
    let dx = x - origin_x;
    let dy = y - origin_y;
    dx * dx + dy * dy > DRAG_SLOP * DRAG_SLOP
}

fn note_move(drag: &mut DragState, event: PointerEvent) -> (f32, f32) {
    let dx = event.x - drag.last_x;
    let dy = event.y - drag.last_y;
    drag.previous_move = drag.latest_move;
    drag.latest_move = Some(Sample {
        x: event.x,
        y: event.y,
        time_ms: event.time_ms,
    });
    drag.last_x = event.x;
    drag.last_y = event.y;
    (dx, dy)
}

fn drag_velocity(drag: &DragState) -> (f32, f32) {
    let (Some(previous), Some(latest)) = (drag.previous_move, drag.latest_move) else {
        return (0.0, 0.0);
    };
    let dt = latest.time_ms - previous.time_ms;
    if !dt.is_finite() || dt <= 0.0 {
        return (0.0, 0.0);
    }
    (
        (f64::from(latest.x - previous.x) / dt) as f32,
        (f64::from(latest.y - previous.y) / dt) as f32,
    )
}

fn justify_content(justify: Justify) -> JustifyContent {
    match justify {
        Justify::Start => JustifyContent::Start,
        Justify::Center => JustifyContent::Center,
        Justify::End => JustifyContent::End,
        Justify::SpaceBetween => JustifyContent::SpaceBetween,
        Justify::SpaceAround => JustifyContent::SpaceAround,
        Justify::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

fn scroll_style() -> Style {
    Style {
        display: Display::Grid,
        grid_template_columns: vec![max_content()],
        grid_template_rows: vec![max_content()],
        align_self: Some(AlignSelf::Stretch),
        flex_grow: 1.0,
        flex_shrink: 1.0,
        min_size: Size {
            width: length(0.0),
            height: length(0.0),
        },
        overflow: taffy::Point {
            x: taffy::style::Overflow::Hidden,
            y: taffy::style::Overflow::Hidden,
        },
        ..Style::default()
    }
}

fn stack_style() -> Style {
    Style {
        display: Display::Grid,
        grid_template_columns: vec![min_content()],
        grid_template_rows: vec![min_content()],
        ..Style::default()
    }
}

fn flex_style(direction: FlexDirection, gap: f32) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: direction,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        align_items: Some(AlignItems::Start),
        justify_content: Some(JustifyContent::Start),
        ..Style::default()
    }
}

fn line_limit(space: AvailableSpace) -> f32 {
    match space {
        AvailableSpace::Definite(width) if width > 0.0 => width,
        AvailableSpace::Definite(_) => 0.0,
        AvailableSpace::MinContent | AvailableSpace::MaxContent => SINGLE_LINE_MAX,
    }
}

fn measure_leaf(
    slots: &[Slot],
    shaper: &dyn Shaper,
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    context: Option<&mut usize>,
) -> Size<f32> {
    let Some(index) = context.copied() else {
        return Size::ZERO;
    };
    let Some(slot) = slots.get(index) else {
        return Size::ZERO;
    };
    let Some(node) = slot.node.as_ref() else {
        return Size::ZERO;
    };
    let size = if let Some(text) = &node.text {
        let max_width = line_limit(available.width);
        text.max_width.set(max_width);
        shaper.measure(
            &text.utf8,
            text.font_size,
            max_width,
            text.color,
            &text.style,
        )
    } else if let Some(image) = &node.image {
        TextSize {
            width: image.width,
            height: image.height,
        }
    } else {
        TextSize {
            width: 0.0,
            height: 0.0,
        }
    };
    Size {
        width: known.width.unwrap_or(size.width),
        height: known.height.unwrap_or(size.height),
    }
}

#[cfg(test)]
mod slot_tests {
    use super::{MAX_GENERATION, Tree};
    use crate::{Error, MonoShaper};

    #[test]
    fn recycled_slot_rejects_the_previous_id() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper);
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let stale = text;
        tree.retire(text.index);
        let reused = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_eq!(reused.index, stale.index);
        assert!(matches!(
            tree.set_text(stale, "no"),
            Err(Error::UnknownNode)
        ));
        tree.set_text(reused, "Ok")?;
        Ok(())
    }

    #[test]
    fn generation_cap_retires_the_slot() -> Result<(), Error> {
        let mut tree = Tree::new(MonoShaper);
        let text = tree.text("Hi", 16.0, [0, 0, 0, 255])?;
        let index = text.index;
        tree.slots[index as usize].generation = MAX_GENERATION;
        tree.retire(index);
        assert!(tree.free.is_empty());
        let next = tree.text("Yo", 16.0, [0, 0, 0, 255])?;
        assert_ne!(next.index, index);
        Ok(())
    }
}
