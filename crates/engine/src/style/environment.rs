//! Theme, window, and interaction inputs that class variants read.
//!
//! The session owns one environment. `uicore` does not import solid-ui types.

use std::collections::HashMap;
use std::sync::Arc;

use super::StyleError;
use super::values::PaintColor;

/// Bytes of one class string. Longer input is rejected before parsing.
pub const CLASS_BYTE_LIMIT: usize = 8 * 1024;

/// Default breakpoints in window scene points: sm, md, lg, xl, 2xl.
pub const BREAKPOINTS: [f32; 5] = [640.0, 768.0, 1024.0, 1280.0, 1536.0];

bitflags::bitflags! {
    /// Which environment changes must re-resolve a node.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct DepMask: u16 {
        const WINDOW = 1 << 0;
        const DARK = 1 << 1;
        const HOVER = 1 << 2;
        const ACTIVE = 1 << 3;
        const FOCUS = 1 << 4;
        const DISABLED = 1 << 5;
        const THEME = 1 << 6;
    }
}

/// Pointer and focus state already computed by the tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    pub hover: bool,
    pub active: bool,
    pub focus: bool,
    pub disabled: bool,
}

/// One semantic type role. `line_height` is a multiplier, not points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeRole {
    pub size: f32,
    pub weight: u16,
    pub line_height: f32,
}

/// A length after the environment has been applied. Fractions stay for Taffy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolvedLength {
    Points(f32),
    /// Parent fraction. `1.0` is 100%.
    Fraction(f32),
}

/// Window width, palette, and type roles used while selecting class variants.
#[derive(Clone, Debug)]
pub struct Environment {
    pub window_width: f32,
    pub window_height: f32,
    /// Scene points in one spacing step. `p-4` is four steps.
    pub spacing: f32,
    pub dark: bool,
    /// Default direction inherited by scene roots. Local declarations take precedence.
    pub direction: super::DirectionMode,
    pub revision: u64,
    pub root_rem: f32,
    pub breakpoints: [f32; 5],
    colors: HashMap<Arc<str>, [u8; 4]>,
    pub radii: Radii,
    pub font_sizes: FontSizes,
    roles: HashMap<&'static str, TypeRole>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub none: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xl2: f32,
    pub full: f32,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            none: 0.0,
            sm: 2.0,
            md: 6.0,
            lg: 8.0,
            xl: 12.0,
            xl2: 16.0,
            full: 9999.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSizes {
    pub xs: f32,
    pub sm: f32,
    pub base: f32,
    pub lg: f32,
    pub xl: f32,
    pub xl2: f32,
    pub xl3: f32,
    pub xl4: f32,
}

impl Default for FontSizes {
    fn default() -> Self {
        Self {
            xs: 12.0,
            sm: 14.0,
            base: 16.0,
            lg: 18.0,
            xl: 20.0,
            xl2: 24.0,
            xl3: 30.0,
            xl4: 36.0,
        }
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::light()
    }
}

impl Environment {
    pub fn validate(&self) -> Result<(), StyleError> {
        let values = [
            self.window_width,
            self.window_height,
            self.spacing,
            self.root_rem,
        ];
        if values
            .iter()
            .chain(self.breakpoints.iter())
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(StyleError::Invalid {
                node: None,
                property: "environment",
                reason: "environment lengths must be finite and non-negative",
            });
        }
        Ok(())
    }

    pub(crate) fn same_theme(&self, other: &Self) -> bool {
        self.colors == other.colors
            && self.radii == other.radii
            && self.font_sizes == other.font_sizes
            && self.roles == other.roles
            && self.revision == other.revision
            && self.direction == other.direction
    }
    pub fn light() -> Self {
        let mut env = Self::blank(false);
        env.install_palette(LIGHT_PALETTE);
        env
    }

    pub fn dark() -> Self {
        let mut env = Self::blank(true);
        env.install_palette(DARK_PALETTE);
        env
    }

    fn blank(dark: bool) -> Self {
        let mut roles = HashMap::new();
        roles.insert("display", role(32.0, 600));
        roles.insert("headline", role(24.0, 600));
        roles.insert("title", role(20.0, 600));
        roles.insert("body", role(16.0, 400));
        roles.insert("label", role(14.0, 500));
        roles.insert("caption", role(12.0, 400));
        Self {
            window_width: 360.0,
            window_height: 640.0,
            spacing: 4.0,
            dark,
            direction: super::DirectionMode::Ltr,
            revision: 1,
            root_rem: 16.0,
            breakpoints: BREAKPOINTS,
            colors: HashMap::new(),
            radii: Radii::default(),
            font_sizes: FontSizes::default(),
            roles,
        }
    }

    pub fn install_palette(&mut self, entries: &[(&str, [u8; 4])]) {
        self.colors.clear();
        for (name, color) in entries {
            self.colors.insert(Arc::from(*name), *color);
        }
        if let Some(text) = self.colors.get("text").copied() {
            self.colors.insert(Arc::from("foreground"), text);
        }
        self.colors.insert(Arc::from("black"), [0, 0, 0, 255]);
        self.colors.insert(Arc::from("white"), [255, 255, 255, 255]);
        self.colors.insert(Arc::from("transparent"), [0, 0, 0, 0]);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn set_role(&mut self, name: &'static str, role: TypeRole) {
        self.roles.insert(name, role);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn color(&self, name: &str) -> Option<[u8; 4]> {
        self.colors.get(name).copied()
    }

    pub fn resolve_length(
        &self,
        length: &super::values::Length,
    ) -> Result<ResolvedLength, StyleError> {
        use super::values::Length;
        Ok(match length {
            Length::Points(value) => ResolvedLength::Points(*value),
            Length::Fraction(value) => ResolvedLength::Fraction(*value),
            Length::Rem(value) => ResolvedLength::Points(value * self.root_rem),
            Length::ViewportWidth(value) => ResolvedLength::Points(value * self.window_width),
            Length::ViewportHeight(value) => ResolvedLength::Points(value * self.window_height),
            Length::Steps(value) => ResolvedLength::Points(value * self.spacing),
            Length::Token(token) => ResolvedLength::Points(self.length_token(token.as_str())?),
        })
    }

    pub fn resolve_points(&self, length: &super::values::Length) -> Result<f32, StyleError> {
        match self.resolve_length(length)? {
            ResolvedLength::Points(value) if value.is_finite() => Ok(value),
            ResolvedLength::Points(_) => Err(StyleError::Invalid {
                node: None,
                property: "length",
                reason: "resolved length is not finite",
            }),
            ResolvedLength::Fraction(_) => Err(StyleError::Invalid {
                node: None,
                property: "length",
                reason: "percentage is not a point length here",
            }),
        }
    }

    pub fn resolve_font_size(
        &self,
        length: &super::values::AbsoluteLength,
    ) -> Result<f32, StyleError> {
        use super::values::AbsoluteLength;
        match length {
            AbsoluteLength::Points(value) => Ok(*value),
            AbsoluteLength::Rem(value) => Ok(value * self.root_rem),
            AbsoluteLength::ViewportWidth(value) => Ok(value * self.window_width),
            AbsoluteLength::ViewportHeight(value) => Ok(value * self.window_height),
            AbsoluteLength::Token(token) => self.font_token(token.as_str()),
        }
    }

    fn length_token(&self, name: &str) -> Result<f32, StyleError> {
        let radius = match name {
            "radius.none" => Some(self.radii.none),
            "radius.sm" => Some(self.radii.sm),
            "radius.md" => Some(self.radii.md),
            "radius.lg" => Some(self.radii.lg),
            "radius.xl" => Some(self.radii.xl),
            "radius.2xl" => Some(self.radii.xl2),
            "radius.full" => Some(self.radii.full),
            _ => None,
        };
        radius.ok_or(StyleError::Invalid {
            node: None,
            property: "length",
            reason: "unknown length token",
        })
    }

    fn font_token(&self, name: &str) -> Result<f32, StyleError> {
        let size = match name {
            "text.xs" => Some(self.font_sizes.xs),
            "text.sm" => Some(self.font_sizes.sm),
            "text.base" => Some(self.font_sizes.base),
            "text.lg" => Some(self.font_sizes.lg),
            "text.xl" => Some(self.font_sizes.xl),
            "text.2xl" => Some(self.font_sizes.xl2),
            "text.3xl" => Some(self.font_sizes.xl3),
            "text.4xl" => Some(self.font_sizes.xl4),
            _ => None,
        };
        size.ok_or(StyleError::Invalid {
            node: None,
            property: "fontSize",
            reason: "unknown font token",
        })
    }

    pub fn resolve_color(&self, color: &PaintColor) -> Result<[u8; 4], StyleError> {
        match color {
            PaintColor::Rgba(rgba) => Ok(*rgba),
            PaintColor::TokenAlpha(name, alpha) => {
                let mut rgba = self.resolve_color(&PaintColor::Token(Arc::clone(name)))?;
                rgba[3] = u8::try_from((u16::from(rgba[3]) * u16::from(*alpha) + 127) / 255)
                    .map_err(|_| StyleError::Invalid {
                        node: None,
                        property: "color",
                        reason: "alpha out of range",
                    })?;
                Ok(rgba)
            }
            PaintColor::Token(name) => self.color(name).ok_or(StyleError::Invalid {
                node: None,
                property: "color",
                reason: "unknown color token",
            }),
        }
    }

    pub fn role(&self, name: &str) -> Option<TypeRole> {
        self.roles.get(name).copied()
    }

    pub fn breakpoint_matches(&self, index: usize) -> bool {
        self.breakpoints
            .get(index)
            .is_some_and(|min| self.window_width >= *min)
    }
}

fn role(size: f32, weight: u16) -> TypeRole {
    TypeRole {
        size,
        weight,
        line_height: 1.25,
    }
}

const LIGHT_PALETTE: &[(&str, [u8; 4])] = &[
    ("bg", hex("#F3F6F4")),
    ("background", hex("#F3F6F4")),
    ("onBackground", hex("#14201A")),
    ("surface", hex("#FFFFFF")),
    ("onSurface", hex("#14201A")),
    ("surfaceContainer", hex("#E5EEE8")),
    ("onSurfaceContainer", hex("#14201A")),
    ("text", hex("#14201A")),
    ("textMuted", hex("#5C6B63")),
    ("accent", hex("#127A32")),
    ("primary", hex("#127A32")),
    ("onPrimary", hex("#FFFFFF")),
    ("primaryContainer", hex("#D7F5E3")),
    ("onPrimaryContainer", hex("#06401C")),
    ("danger", hex("#C83B3B")),
    ("success", hex("#127A32")),
    ("warning", hex("#A56A00")),
    ("focusRing", hex("#127A32")),
    ("scrim", [0, 0, 0, 0x80]),
    ("outline", hex("#5C6B63")),
    ("outlineVariant", hex("#D5E0D9")),
];

const DARK_PALETTE: &[(&str, [u8; 4])] = &[
    ("bg", hex("#101412")),
    ("background", hex("#101412")),
    ("onBackground", hex("#F4F7F5")),
    ("surface", hex("#1C2420")),
    ("onSurface", hex("#F4F7F5")),
    ("surfaceContainer", hex("#27312C")),
    ("onSurfaceContainer", hex("#F4F7F5")),
    ("text", hex("#F4F7F5")),
    ("textMuted", hex("#9AA89F")),
    ("accent", hex("#3DDC84")),
    ("primary", hex("#3DDC84")),
    ("onPrimary", hex("#052E16")),
    ("primaryContainer", hex("#143524")),
    ("onPrimaryContainer", hex("#C8F5D8")),
    ("danger", hex("#FF6B6B")),
    ("success", hex("#3DDC84")),
    ("warning", hex("#E6B15A")),
    ("focusRing", hex("#3DDC84")),
    ("scrim", [0, 0, 0, 0x99]),
    ("outline", hex("#6D7C74")),
    ("outlineVariant", hex("#2C3832")),
];

const fn hex(text: &str) -> [u8; 4] {
    let bytes = text.as_bytes();
    let r = hex_byte(bytes[1], bytes[2]);
    let g = hex_byte(bytes[3], bytes[4]);
    let b = hex_byte(bytes[5], bytes[6]);
    [r, g, b, 255]
}

const fn hex_byte(hi: u8, lo: u8) -> u8 {
    hex_nibble(hi) * 16 + hex_nibble(lo)
}

const fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => 0,
    }
}
