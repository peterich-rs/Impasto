//! Compiler wire format for batched scene creation.

use serde::{Deserialize, Serialize};

use crate::style::static_pool;

/// The native node and its component layout preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateKind {
    View,
    Stack,
    Scroll,
    Text,
    Row,
    Column,
    Grid,
    RepaintBoundary,
    Image,
}

impl TemplateKind {
    /// Read a tag from a compiler template or a legacy JavaScript row.
    pub fn parse(tag: &str) -> Option<Self> {
        Some(match tag {
            "view" => Self::View,
            "stack" => Self::Stack,
            "scroll" => Self::Scroll,
            "text" => Self::Text,
            "row" => Self::Row,
            "column" => Self::Column,
            "grid" => Self::Grid,
            "repaint" => Self::RepaintBoundary,
            "image" => Self::Image,
            _ => return None,
        })
    }
}

/// One node in a parent-first template. Indices address the template's imports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRow {
    pub kind: TemplateKind,
    pub parent: u8,
    pub text: Option<String>,
    pub style: Option<u8>,
    pub class: Option<u8>,
    pub image: Option<u8>,
}

/// A static image source and its optional intrinsic dimensions and label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateImage {
    pub uri: String,
    pub intrinsic_size_bits: Option<(u32, u32)>,
    pub required: bool,
    pub alt: Option<String>,
}

/// The parent-first node table and image records referenced by its rows.
#[derive(Debug, Serialize, Deserialize)]
pub struct Template {
    pub rows: Vec<TemplateRow>,
    pub images: Vec<TemplateImage>,
}

#[derive(Deserialize)]
struct LegacyRow {
    kind: TemplateKind,
    parent: u8,
    text: Option<String>,
    style: Option<u8>,
    class: Option<u8>,
}

const VERSION: &str = "2:";

/// Encode the validated node records for a single template.
pub fn encode(template: &Template) -> Result<String, String> {
    validate(template)?;
    static_pool::encode_version(template, VERSION)
}

/// Read a template and check its size, parent order and text slots.
pub fn decode(encoded: &str) -> Result<Template, String> {
    let template = if encoded.starts_with(VERSION) {
        static_pool::decode_version(encoded, VERSION)?
    } else {
        let rows: Vec<LegacyRow> = static_pool::decode(encoded)?;
        Template {
            rows: rows
                .into_iter()
                .map(|row| TemplateRow {
                    kind: row.kind,
                    parent: row.parent,
                    text: row.text,
                    style: row.style,
                    class: row.class,
                    image: None,
                })
                .collect(),
            images: Vec::new(),
        }
    };
    validate(&template)?;
    Ok(template)
}

fn validate(Template { rows, images }: &Template) -> Result<(), String> {
    if rows.is_empty() || rows.len() > 256 {
        return Err("invalid template size".to_owned());
    }
    for (index, row) in rows.iter().enumerate() {
        let parent = usize::from(row.parent);
        if (index == 0 && parent != 0) || (index != 0 && parent >= index) {
            return Err("invalid template parent".to_owned());
        }
        if (row.kind == TemplateKind::Text) != row.text.is_some() {
            return Err("invalid template text slot".to_owned());
        }
        if row.image.is_some_and(|slot| {
            row.kind != TemplateKind::Image || usize::from(slot) >= images.len()
        }) {
            return Err("invalid template image slot".to_owned());
        }
    }
    if images.len() > 256
        || images.iter().any(|image| {
            image.intrinsic_size_bits.is_some_and(|(width, height)| {
                [f32::from_bits(width), f32::from_bits(height)]
                    .into_iter()
                    .any(|value| !value.is_finite() || value < 0.)
            })
        })
    {
        return Err("invalid template image record".to_owned());
    }
    Ok(())
}
