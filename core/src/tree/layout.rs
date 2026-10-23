//! Taffy styles, measure, and the viewport.

use protocol::{TextAlign, TextStyle};
use taffy::NodeId as TaffyId;
use taffy::prelude::*;

use super::node::{Kind, Slot, TextNode};
use super::{Alignment, Edges, Justify, NodeId, Rect, TrackSize, Tree};
use crate::{Error, SINGLE_LINE_MAX, Shaper, TextSize};

impl Tree {
    /// Content box for the next `layout`, in scene points.
    ///
    /// `(x, y)` is where the root is placed. `width` and `height` are the space
    /// Taffy may use. Unset size keeps `MAX_CONTENT`. Does not drop the previous
    /// geometry. Hit testing uses that geometry until `layout` runs again.
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32) -> Result<(), Error> {
        finite_number(x)?;
        finite_number(y)?;
        finite_number(width)?;
        finite_number(height)?;
        self.origin = (x, y);
        self.viewport = Some((width.max(0.0), height.max(0.0)));
        self.dirty = true;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
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
        self.set_padding_edges(id, Edges::all(edges))
    }

    /// Padding on a [`Kind::Padding`] node. A non-finite edge leaves the style unchanged.
    pub fn set_padding_edges(&mut self, id: NodeId, edges: Edges) -> Result<(), Error> {
        let padding = edge_rect(edges)?;
        if self.node(id)?.kind != Kind::Padding {
            return Err(Error::WrongKind);
        }
        let mut style = self.node(id)?.style.clone();
        style.padding = padding;
        self.write_style(id, style)
    }

    /// Margin on any node, in scene points.
    pub fn set_margin(&mut self, id: NodeId, edges: Edges) -> Result<(), Error> {
        let margin = margin_rect(edges)?;
        let mut style = self.node(id)?.style.clone();
        style.margin = margin;
        self.write_style(id, style)
    }

    pub fn set_min_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.min_size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)
    }

    pub fn set_max_size(&mut self, id: NodeId, width: f32, height: f32) -> Result<(), Error> {
        let width = finite_positive(width)?;
        let height = finite_positive(height)?;
        let mut style = self.node(id)?.style.clone();
        style.max_size = Size {
            width: length(width),
            height: length(height),
        };
        self.write_style(id, style)
    }

    pub fn set_shrink(&mut self, id: NodeId, shrink: f32) -> Result<(), Error> {
        finite_number(shrink)?;
        if shrink < 0.0 {
            return Err(Error::BadNumber);
        }
        let mut style = self.node(id)?.style.clone();
        style.flex_shrink = shrink;
        self.write_style(id, style)
    }

    pub fn set_basis(&mut self, id: NodeId, basis: f32) -> Result<(), Error> {
        let basis = finite_positive(basis)?;
        let mut style = self.node(id)?.style.clone();
        style.flex_basis = length(basis);
        self.write_style(id, style)
    }

    pub fn set_align_self(&mut self, id: NodeId, alignment: Alignment) -> Result<(), Error> {
        let mut style = self.node(id)?.style.clone();
        style.align_self = Some(align_self(alignment));
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

    pub(super) fn edit_text(
        &mut self,
        id: NodeId,
        edit: impl FnOnce(&mut TextNode),
    ) -> Result<(), Error> {
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

    pub(super) fn write_style(&mut self, id: NodeId, style: Style) -> Result<(), Error> {
        let taffy_id = self.node(id)?.taffy;
        self.node_mut(id)?.style = style.clone();
        if let Some(taffy_id) = taffy_id {
            self.taffy.set_style(taffy_id, style)?;
        }
        self.dirty = true;
        Ok(())
    }

    pub(super) fn touch_measure(&mut self, taffy_id: Option<TaffyId>) -> Result<(), Error> {
        if let Some(taffy_id) = taffy_id {
            self.taffy.mark_dirty(taffy_id)?;
        }
        self.dirty = true;
        Ok(())
    }

    pub(super) fn place(&mut self, id: NodeId, origin_x: f32, origin_y: f32) -> Result<(), Error> {
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
}

pub(super) fn finite_number(value: f32) -> Result<(), Error> {
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

pub(super) fn align_axes(alignment: Alignment) -> (AlignItems, JustifyContent) {
    match alignment {
        Alignment::Start => (AlignItems::Start, JustifyContent::Start),
        Alignment::Center => (AlignItems::Center, JustifyContent::Center),
        Alignment::End => (AlignItems::End, JustifyContent::End),
    }
}

fn edge_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentage>, Error> {
    finite_number(edges.top)?;
    finite_number(edges.right)?;
    finite_number(edges.bottom)?;
    finite_number(edges.left)?;
    Ok(taffy::Rect {
        left: length(edges.left),
        right: length(edges.right),
        top: length(edges.top),
        bottom: length(edges.bottom),
    })
}

fn margin_rect(edges: Edges) -> Result<taffy::Rect<LengthPercentageAuto>, Error> {
    finite_number(edges.top)?;
    finite_number(edges.right)?;
    finite_number(edges.bottom)?;
    finite_number(edges.left)?;
    Ok(taffy::Rect {
        left: length(edges.left),
        right: length(edges.right),
        top: length(edges.top),
        bottom: length(edges.bottom),
    })
}

fn align_self(alignment: Alignment) -> AlignSelf {
    match alignment {
        Alignment::Start => AlignSelf::Start,
        Alignment::Center => AlignSelf::Center,
        Alignment::End => AlignSelf::End,
    }
}

pub(super) fn tracks<T>(sizes: &[TrackSize]) -> Result<Vec<T>, Error>
where
    T: TaffyMinContent + TaffyMaxContent + FromLength + FromFr,
{
    let mut out = Vec::with_capacity(sizes.len());
    for size in sizes {
        out.push(match *size {
            TrackSize::MinContent => T::MIN_CONTENT,
            TrackSize::MaxContent => T::MAX_CONTENT,
            TrackSize::Points(value) => {
                finite_positive(value)?;
                T::from_length(value)
            }
            TrackSize::Fraction(value) => {
                finite_number(value)?;
                if value < 0.0 {
                    return Err(Error::BadNumber);
                }
                T::from_fr(value)
            }
        });
    }
    Ok(out)
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

pub(super) fn scroll_style() -> Style {
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

pub(super) fn stack_style() -> Style {
    Style {
        display: Display::Grid,
        grid_template_columns: vec![min_content()],
        grid_template_rows: vec![min_content()],
        ..Style::default()
    }
}

pub(super) fn flex_style(direction: FlexDirection, gap: f32) -> Style {
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
