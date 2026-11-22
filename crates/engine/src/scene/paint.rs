/// Color space of one gradient stop. Only sRGB is defined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ColorSpace {
    #[default]
    Srgb,
}

/// One color stop on a gradient. `offset` is in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f32,
    pub color: [u8; 4],
    pub space: ColorSpace,
}

impl ColorStop {
    pub const fn srgb(offset: f32, color: [u8; 4]) -> Self {
        Self {
            offset,
            color,
            space: ColorSpace::Srgb,
        }
    }
}

/// Fill gradient in the rectangle's unit square. `(0, 0)` is the top-left.
/// Sweep angles are radians, clockwise from the positive x axis.
#[derive(Clone, Debug, PartialEq)]
pub enum Gradient {
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        stops: Vec<ColorStop>,
    },
    Radial {
        center: [f32; 2],
        radius: f32,
        stops: Vec<ColorStop>,
    },
    Sweep {
        center: [f32; 2],
        start_angle_rad: f32,
        end_angle_rad: f32,
        stops: Vec<ColorStop>,
    },
}

/// Material shadow. `elevation` is scene points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: [u8; 4],
    pub elevation: f32,
}

/// A drop shadow drawn in the same picture as its rectangle.
///
/// `offset` and `blur_sigma` are scene points. The player does not multiply
/// `blur_sigma` by `point_scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset: [f32; 2],
    pub blur_sigma: f32,
    pub spread: f32,
    pub color: [u8; 4],
}

/// Layer blend. [`BlendMode::SrcOver`] is a no-op at record time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    SrcOver,
    Multiply,
    Screen,
}
