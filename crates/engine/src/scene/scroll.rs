//! Scroll rules shared by the UI and compositor queues.

use super::CompositorProp;

/// An independent channel of the scroll geometry, in scene points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollSource {
    X,
    Y,
    OverscrollX,
    OverscrollY,
}

impl ScrollSource {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "scrollX" => Some(Self::X),
            "scrollY" => Some(Self::Y),
            "overscrollX" => Some(Self::OverscrollX),
            "overscrollY" => Some(Self::OverscrollY),
            _ => None,
        }
    }

    pub const fn prop(self) -> CompositorProp {
        match self {
            Self::X => CompositorProp::ScrollX,
            Self::Y => CompositorProp::ScrollY,
            Self::OverscrollX => CompositorProp::OverscrollX,
            Self::OverscrollY => CompositorProp::OverscrollY,
        }
    }
}

/// A discrete observation of one scroll position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollThreshold {
    pub source: ScrollSource,
    pub value: f32,
    pub from_end: bool,
    pub hysteresis: f32,
}

impl ScrollThreshold {
    pub fn is_valid(self) -> bool {
        self.value.is_finite()
            && self.hysteresis.is_finite()
            && self.hysteresis >= 0.0
            && (self.value.abs() + self.hysteresis).is_finite()
            && !(self.from_end
                && matches!(
                    self.source,
                    ScrollSource::OverscrollX | ScrollSource::OverscrollY
                ))
    }

    fn descending(self) -> bool {
        !self.from_end
            && self.value < 0.0
            && matches!(
                self.source,
                ScrollSource::OverscrollX | ScrollSource::OverscrollY
            )
    }

    fn boundary(self, limit: f32, active: Option<bool>) -> f32 {
        let start = if self.from_end {
            (limit - self.value.abs()).max(0.0)
        } else if self.descending() {
            -self.value
        } else {
            self.value
        };
        match active {
            Some(true) => start - self.hysteresis,
            Some(false) if self.from_end => (start + self.hysteresis).min(limit),
            Some(false) => start + self.hysteresis,
            None => start,
        }
    }

    pub fn evaluate(self, value: f32, limit: f32, active: Option<bool>) -> bool {
        let signed = if self.descending() { -value } else { value };
        signed >= self.boundary(limit, active)
    }

    /// Half-open interval of offsets that leave the active state unchanged.
    pub fn stable_range(self, limit: f32, active: bool) -> [f32; 2] {
        let edge = self.boundary(limit, Some(active));
        match (self.descending(), active) {
            (false, false) => [f32::NEG_INFINITY, edge],
            (false, true) => [edge, f32::INFINITY],
            (true, false) => [(-edge).next_up(), f32::INFINITY],
            (true, true) => [f32::NEG_INFINITY, (-edge).next_up()],
        }
    }
}

/// How an interpolation behaves outside the provided input range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extrapolate {
    Clamp,
    Extend,
}

/// A finite, strictly increasing set of input stops and their matching values.
#[derive(Clone, Debug, PartialEq)]
pub struct Piecewise<T> {
    inputs: Box<[f32]>,
    outputs: Box<[T]>,
    extrapolate: Extrapolate,
}

pub type PiecewiseMap = Piecewise<f32>;
/// Outputs are linear, premultiplied RGBA.
pub type PiecewiseColorMap = Piecewise<[f32; 4]>;

impl<T> Piecewise<T> {
    pub fn new(inputs: Box<[f32]>, outputs: Box<[T]>, extrapolate: Extrapolate) -> Option<Self> {
        (inputs.len() >= 2
            && inputs.len() == outputs.len()
            && inputs.iter().all(|value| value.is_finite())
            && inputs.windows(2).all(|pair| pair[0] < pair[1]))
        .then_some(Self {
            inputs,
            outputs,
            extrapolate,
        })
    }

    fn interpolate(&self, input: f32, mix: impl Fn(&T, &T, f64) -> T) -> T {
        let input = match self.extrapolate {
            Extrapolate::Clamp => input.clamp(self.inputs[0], self.inputs[self.inputs.len() - 1]),
            Extrapolate::Extend => input,
        };
        let left = self
            .inputs
            .partition_point(|value| *value <= input)
            .saturating_sub(1)
            .min(self.inputs.len() - 2);
        let low = f64::from(self.inputs[left]);
        let high = f64::from(self.inputs[left + 1]);
        let t = (f64::from(input) - low) / (high - low);
        mix(&self.outputs[left], &self.outputs[left + 1], t)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScrollCurve {
    Scalar(PiecewiseMap),
    Color(PiecewiseColorMap),
    Linear {
        factor: f32,
        offset: f32,
        maximum: Option<f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollValue {
    Scalar(f32),
    Alpha(u8),
    Color([f32; 4]),
}

impl ScrollCurve {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Scalar(map) => map.outputs.iter().all(|value| value.is_finite()),
            Self::Color(map) => map.outputs.iter().all(|[r, g, b, a]| {
                (0.0..=1.0).contains(a)
                    && [r, g, b]
                        .into_iter()
                        .all(|value| (0.0..=*a).contains(value))
            }),
            Self::Linear {
                factor,
                offset,
                maximum,
            } => factor.is_finite() && offset.is_finite() && maximum.is_none_or(f32::is_finite),
        }
    }

    pub fn sample(&self, input: f32) -> ScrollValue {
        let mix = |a: &f32, b: &f32, t: f64| {
            let value = f64::from(*a) + (f64::from(*b) - f64::from(*a)) * t;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "scene properties use finite f32 values"
            )]
            let value = value.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
            value
        };
        match self {
            Self::Scalar(map) => ScrollValue::Scalar(map.interpolate(input, mix)),
            Self::Color(map) => ScrollValue::Color(map.interpolate(input, |a, b, t| {
                let alpha = mix(&a[3], &b[3], t).clamp(0.0, 1.0);
                let [r, g, b] =
                    [0, 1, 2].map(|index| mix(&a[index], &b[index], t).clamp(0.0, alpha));
                [r, g, b, alpha]
            })),
            Self::Linear {
                factor,
                offset,
                maximum,
            } => {
                let value = input * factor + offset;
                let value = value.clamp(-f32::MAX, f32::MAX);
                ScrollValue::Scalar(maximum.map_or(value, |limit| value.min(limit)))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollTargetProp {
    TranslateX,
    TranslateY,
    ScaleX,
    ScaleY,
    Scale,
    Rotate,
    Opacity,
    BlurSigma,
    BackdropBlurSigma,
    BackgroundColor,
    ShaderUniform(u8),
    ScrollX,
    ScrollY,
}

impl ScrollTargetProp {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "translateX" => Self::TranslateX,
            "translateY" => Self::TranslateY,
            "scaleX" => Self::ScaleX,
            "scaleY" => Self::ScaleY,
            "scale" => Self::Scale,
            "rotate" => Self::Rotate,
            "opacity" => Self::Opacity,
            "blur" => Self::BlurSigma,
            "backdropBlur" => Self::BackdropBlurSigma,
            "backgroundColor" => Self::BackgroundColor,
            "scrollX" => Self::ScrollX,
            "scrollY" => Self::ScrollY,
            name => {
                let index = name.strip_prefix("shaderUniform:")?.parse().ok()?;
                if usize::from(index) >= super::MAX_SHADER_UNIFORM_BYTES / size_of::<f32>() {
                    return None;
                }
                Self::ShaderUniform(index)
            }
        })
    }

    pub fn evaluate(self, curve: &ScrollCurve, input: f32) -> ScrollValue {
        match (self, curve.sample(input)) {
            (Self::Opacity, ScrollValue::Scalar(value)) => {
                ScrollValue::Alpha(super::compositor_alpha(value.clamp(0.0, 1.0) * 255.0))
            }
            (Self::BlurSigma | Self::BackdropBlurSigma, ScrollValue::Scalar(value)) => {
                ScrollValue::Scalar(value.max(0.0))
            }
            (_, value) => value,
        }
    }

    pub fn is_transform(self) -> bool {
        matches!(
            self,
            Self::TranslateX
                | Self::TranslateY
                | Self::ScaleX
                | Self::ScaleY
                | Self::Scale
                | Self::Rotate
        )
    }

    pub fn scroll_channel(self) -> Option<ScrollSource> {
        match self {
            Self::ScrollX => Some(ScrollSource::X),
            Self::ScrollY => Some(ScrollSource::Y),
            _ => None,
        }
    }

    pub fn apply_transform(self, transform: &mut super::CompositorTransform, value: f32) {
        match self {
            Self::TranslateX => transform.translate[0] = value,
            Self::TranslateY => transform.translate[1] = value,
            Self::Rotate => transform.rotate_rad = value,
            Self::Scale => transform.scale = [value; 2],
            Self::ScaleX => transform.scale[0] = value,
            Self::ScaleY => transform.scale[1] = value,
            _ => {}
        }
    }
}

/// A rule with its target resolved to a layer of the installed scene.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollBinding {
    pub layer: super::LayerId,
    pub prop: ScrollTargetProp,
    pub curve: std::sync::Arc<ScrollCurve>,
    pub transform: Option<super::CompositorTransform>,
    /// Scroll targets use the layout-time content limit and rubber-band offset.
    pub scroll: Option<[f32; 2]>,
}

impl ScrollBinding {
    pub fn apply(&self, layer: &mut super::LayerKind, value: ScrollValue) -> bool {
        use super::LayerKind;
        match (layer, self.prop, value) {
            (
                LayerKind::Opacity { alpha, .. },
                ScrollTargetProp::Opacity,
                ScrollValue::Alpha(value),
            ) => {
                *alpha = value;
            }
            (
                LayerKind::Blur { sigma, .. },
                ScrollTargetProp::BlurSigma,
                ScrollValue::Scalar(value),
            )
            | (
                LayerKind::BackdropBlur { sigma, .. },
                ScrollTargetProp::BackdropBlurSigma,
                ScrollValue::Scalar(value),
            ) => {
                *sigma = value;
            }
            (
                LayerKind::Fill { color, .. },
                ScrollTargetProp::BackgroundColor,
                ScrollValue::Color(value),
            ) => {
                *color = value;
            }
            (
                LayerKind::Offset { x, .. },
                ScrollTargetProp::ScrollX,
                ScrollValue::Scalar(value),
            )
            | (
                LayerKind::Offset { y: x, .. },
                ScrollTargetProp::ScrollY,
                ScrollValue::Scalar(value),
            ) => {
                let Some([limit, over]) = self.scroll else {
                    return false;
                };
                *x = -(value.clamp(0.0, limit) + over);
            }
            (
                LayerKind::Picture { .. },
                ScrollTargetProp::ShaderUniform(_),
                ScrollValue::Scalar(_),
            ) => {}
            _ => return false,
        }
        true
    }
}

/// Quantizes a linear premultiplied color for the legacy flat display list.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the RGBA components are clamped to bytes"
)]
pub(crate) fn linear_color_bytes([r, g, b, a]: [f32; 4]) -> [u8; 4] {
    let encode = |component: f32| {
        let value = if a == 0.0 {
            0.0
        } else {
            (component / a).clamp(0.0, 1.0)
        };
        let value = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (value * 255.0).round() as u8
    };
    [encode(r), encode(g), encode(b), (a * 255.0).round() as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_overscroll_uses_the_same_cutoff_for_ui_and_compositor() {
        let rule = ScrollThreshold {
            source: ScrollSource::OverscrollY,
            value: -64.0,
            from_end: false,
            hysteresis: 4.0,
        };
        for active in [false, true] {
            let [low, high] = rule.stable_range(0.0, active);
            for value in [0.0, -59.0, -60.0, -62.0, -67.0, -68.0, -72.0] {
                assert_eq!(
                    rule.evaluate(value, 0.0, Some(active)) == active,
                    low <= value && value < high,
                    "{value} with {active}",
                );
            }
        }
    }
}
