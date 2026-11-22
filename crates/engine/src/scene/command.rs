use std::sync::Arc;

use super::{
    BlendMode, BoxShadow, ContentMode, Gradient, ParagraphId, Shadow, TextSpan, TextStyle,
};

/// One paint operation. Coordinates are absolute scene points.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Fill {
        color: [u8; 4],
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// Scene points. Zero is a sharp rectangle.
        corner_radius: f32,
        color: [u8; 4],
        /// Scene points. Zero, negative, or non-finite means no stroke.
        stroke_width: f32,
        stroke_color: [u8; 4],
        gradient: Option<Gradient>,
        shadow: Option<Shadow>,
        /// Drawn before the rectangle, in array order. Empty means no box shadow.
        box_shadows: Arc<[BoxShadow]>,
    },
    Text {
        x: f32,
        y: f32,
        /// Paragraph constraint used when this run was measured, in scene points.
        max_width: f32,
        font_size: f32,
        /// Retained paragraph. [`ParagraphId::NONE`] rebuilds from the payload.
        paragraph: ParagraphId,
        color: [u8; 4],
        utf8: Arc<str>,
        style: Arc<TextStyle>,
        /// Empty means one run with the command color and style.
        spans: Arc<[TextSpan]>,
    },
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        source: Arc<str>,
        mode: ContentMode,
        /// Missing required images fail the frame. Optional images are skipped.
        required: bool,
    },
    /// Later commands are clipped to this rounded rectangle until the matching [`Command::Pop`].
    PushClip {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    },
    /// Later commands are composited offscreen and drawn back at `alpha` until [`Command::Pop`].
    ///
    /// The box is the layer bounds. `alpha` is 0–255, the same range as a color channel.
    PushOpacity {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: u8,
    },
    /// Later commands are scaled, rotated clockwise, then translated until [`Command::Pop`].
    ///
    /// `rotate_rad` is clockwise radians. The player still calls Translate, Rotate
    /// (degrees), Scale in that order.
    PushTransform {
        translate_x: f32,
        translate_y: f32,
        rotate_rad: f32,
        scale_x: f32,
        scale_y: f32,
    },
    /// Blur the layer contents. `sigma` is scene points. Non-positive or non-finite
    /// values are ignored: no push and no matching pop.
    PushBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    /// Blur the backdrop behind this layer. `sigma` is scene points.
    PushBackdropBlur {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sigma: f32,
    },
    /// Composite this layer with Multiply or Screen. [`BlendMode::SrcOver`] is a no-op.
    PushBlend {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        mode: BlendMode,
    },
    /// Closes the most recent push. Invalid blur and SrcOver blend do not push,
    /// so they also do not consume a pop at record time.
    Pop,
    /// A canvas content box. `x` and `y` are the picture origin of that box.
    /// Verbs inside `list` stay in canvas-local coordinates.
    Canvas {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        list: crate::scene::CanvasList,
    },
}

/// Flattened commands for tests and diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub commands: Vec<Command>,
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use super::{Command, DrawList};
    use crate::scene::{ColorStop, Gradient, SpanPaint, TextSpan};

    #[test]
    fn an_unbalanced_pop_is_stored_as_data() {
        let list = DrawList {
            commands: vec![Command::Pop],
        };
        assert_eq!(list.commands, vec![Command::Pop]);
    }

    fn bits(paint: &SpanPaint) -> u64 {
        let mut hasher = DefaultHasher::new();
        paint.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn a_gradient_span_hashes_negative_zero_with_zero() {
        let stops = vec![
            ColorStop::srgb(0.0, [0, 0, 0, 255]),
            ColorStop::srgb(1.0, [255, 0, 0, 255]),
        ];
        let left = TextSpan {
            utf8: "A".into(),
            color: [1, 2, 3, 4],
            gradient: Some(Gradient::Linear {
                start: [-0.0, 0.0],
                end: [1.0, 0.0],
                stops: stops.clone(),
            }),
            weight: 400,
            underline: false,
            line_through: false,
            family: None,
        };
        let right = TextSpan {
            gradient: Some(Gradient::Linear {
                start: [0.0, 0.0],
                end: [1.0, -0.0],
                stops,
            }),
            ..left.clone()
        };
        assert_eq!(left.paint(), right.paint());
        assert_eq!(bits(&left.paint()), bits(&right.paint()));
        let solid = TextSpan {
            gradient: None,
            ..left
        };
        assert_ne!(solid.paint(), right.paint());
    }
}
