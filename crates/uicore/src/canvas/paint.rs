//! Paint sources. Gradients keep authoring coordinates until prepare.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use super::{CanvasError, CanvasErrorCode, MAX_STOPS, finite};

/// One gradient stop in authoring order. Offsets are `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorStop {
    pub offset: f64,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum GradientKind {
    Linear {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Radial {
        x0: f64,
        y0: f64,
        r0: f64,
        x1: f64,
        y1: f64,
        r1: f64,
    },
    Conic {
        angle: f64,
        x: f64,
        y: f64,
    },
}

/// Immutable gradient captured by one drawing call.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientSnap {
    pub kind: GradientKind,
    pub stops: Arc<[ColorStop]>,
}

/// Fill or stroke source stored on the recorder.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintSource {
    Color([u8; 4]),
    Gradient(GradientSnap),
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Linear {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Radial {
        x0: f64,
        y0: f64,
        r0: f64,
        x1: f64,
        y1: f64,
        r1: f64,
    },
    Conic {
        angle: f64,
        x: f64,
        y: f64,
    },
}

#[derive(Debug)]
struct GradientBody {
    kind: Kind,
    stops: Vec<ColorStop>,
    generation: u64,
}

/// A reusable gradient. Copies share the stop list. Each drawing call snapshots it.
#[derive(Clone, Debug)]
pub struct Gradient {
    body: Rc<RefCell<GradientBody>>,
}

impl Gradient {
    pub fn linear(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Self, CanvasError> {
        if ![x0, y0, x1, y1].iter().all(|value| finite(*value)) {
            return Err(CanvasError::argument(
                "createLinearGradient",
                "non-finite coordinate",
            ));
        }
        Ok(Self::from_kind(Kind::Linear { x0, y0, x1, y1 }))
    }

    pub fn radial(
        x0: f64,
        y0: f64,
        r0: f64,
        x1: f64,
        y1: f64,
        r1: f64,
    ) -> Result<Self, CanvasError> {
        if r0 < 0.0 || r1 < 0.0 {
            return Err(CanvasError::argument(
                "createRadialGradient",
                "negative radius",
            ));
        }
        if ![x0, y0, r0, x1, y1, r1].iter().all(|value| finite(*value)) {
            return Err(CanvasError::argument(
                "createRadialGradient",
                "non-finite coordinate",
            ));
        }
        Ok(Self::from_kind(Kind::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
        }))
    }

    pub fn conic(angle: f64, x: f64, y: f64) -> Result<Self, CanvasError> {
        if ![angle, x, y].iter().all(|value| finite(*value)) {
            return Err(CanvasError::argument(
                "createConicGradient",
                "non-finite coordinate",
            ));
        }
        Ok(Self::from_kind(Kind::Conic { angle, x, y }))
    }

    fn from_kind(kind: Kind) -> Self {
        Self {
            body: Rc::new(RefCell::new(GradientBody {
                kind,
                stops: Vec::new(),
                generation: 1,
            })),
        }
    }

    pub fn generation(&self) -> u64 {
        self.body.borrow().generation
    }

    pub fn add_color_stop(&mut self, offset: f64, color: &str) -> Result<(), CanvasError> {
        if !finite(offset) || !(0.0..=1.0).contains(&offset) {
            return Err(CanvasError::argument(
                "addColorStop",
                "offset is outside 0..=1",
            ));
        }
        let mut body = self.body.borrow_mut();
        if body.stops.len() >= MAX_STOPS {
            return Err(CanvasError::limit("addColorStop"));
        }
        let Some(color) = parse_color(color) else {
            return Err(CanvasError::argument("addColorStop", "invalid color"));
        };
        body.stops.push(ColorStop { offset, color });
        body.generation = body.generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> GradientSnap {
        let body = self.body.borrow();
        let mut stops = body.stops.clone();
        stops.sort_by(|left, right| left.offset.total_cmp(&right.offset));
        GradientSnap {
            kind: match body.kind {
                Kind::Linear { x0, y0, x1, y1 } => GradientKind::Linear { x0, y0, x1, y1 },
                Kind::Radial {
                    x0,
                    y0,
                    r0,
                    x1,
                    y1,
                    r1,
                } => GradientKind::Radial {
                    x0,
                    y0,
                    r0,
                    x1,
                    y1,
                    r1,
                },
                Kind::Conic { angle, x, y } => GradientKind::Conic { angle, x, y },
            },
            stops: Arc::from(stops),
        }
    }
}

pub(crate) fn parse_color(input: &str) -> Option<[u8; 4]> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("currentcolor") || lower.contains("var(") {
        return None;
    }
    if let Some(cached) = cached_color(&lower) {
        return Some(cached);
    }
    let parsed = csscolorparser::parse(trimmed).ok()?;
    let color = parsed.to_rgba8();
    store_color(lower, color);
    Some(color)
}

fn cached_color(key: &str) -> Option<[u8; 4]> {
    COLOR_CACHE.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, color)| *color)
    })
}

fn store_color(key: String, color: [u8; 4]) {
    COLOR_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 256 {
            cache.remove(0);
        }
        cache.push((key, color));
    });
}

thread_local! {
    static COLOR_CACHE: RefCell<Vec<(String, [u8; 4])>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn alpha_byte(alpha: f64) -> u8 {
    let scaled = (alpha.clamp(0.0, 1.0) * 255.0).round();
    if !scaled.is_finite() {
        return 255;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "globalAlpha is clamped to 0..=1 before scaling into a byte"
    )]
    {
        scaled as u8
    }
}

pub(crate) fn text_unavailable(api: &'static str) -> CanvasError {
    CanvasError::new(
        CanvasErrorCode::TextUnavailable,
        api,
        "text metrics are unavailable",
    )
}
