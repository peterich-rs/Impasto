/// RGB primaries and transfer function of a gradient stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ColorSpace {
    #[default]
    Srgb,
    /// Display P3 primaries with the sRGB transfer function.
    DisplayP3,
    /// Linear sRGB. The byte representation covers zero through SDR white.
    ExtendedLinear,
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
/// Four rows of straight linear RGBA coefficients, with a bias in each row.
pub type ColorMatrix = [[f32; 5]; 4];

/// Blur and color treatment for a first-class backdrop material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropMaterial {
    pub sigma: f32,
    pub saturation: f32,
    pub brightness: f32,
    /// Straight linear RGBA. Alpha controls tint strength.
    pub tint: [f32; 4],
    /// Proportion of the blur that fades into the sharp backdrop at the bottom.
    pub taper: f32,
}

impl BackdropMaterial {
    pub fn is_valid(self) -> bool {
        self.sigma >= 0.0
            && [self.sigma, self.saturation, self.brightness, self.taper]
                .into_iter()
                .chain(self.tint)
                .all(f32::is_finite)
    }
}
