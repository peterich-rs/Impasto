use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use protocol::{TextAlign, TextSpan, TextStyle};

use crate::player::ffi;

pub(crate) struct Shaped {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) paragraph: ffi::ImpellerParagraph,
    pub(crate) used: u64,
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct ParaKey {
    pub(super) utf8: String,
    pub(super) font_size: u32,
    pub(super) max_width: u32,
    pub(super) color: [u8; 4],
    pub(super) family: String,
    pub(super) max_lines: u32,
    pub(super) ellipsis: String,
    pub(super) align: TextAlign,
    pub(super) weight: u16,
    pub(super) direction_rtl: bool,
    pub(super) height_bits: u32,
    pub(super) underline: bool,
    pub(super) line_through: bool,
    pub(super) soft_wrap: bool,
    pub(super) overflow: protocol::TextOverflow,
    pub(super) spans: Vec<SpanKey>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct SpanKey {
    pub(super) utf8: String,
    pub(super) family: String,
    pub(super) weight: u16,
    pub(super) color: [u8; 4],
    pub(super) underline: bool,
    pub(super) line_through: bool,
}

/// Owned keys in hash buckets. A hit compares the query to the stored key and
/// does not allocate the paragraph text.
pub(super) struct ParagraphStore {
    pub(super) buckets: HashMap<u64, Vec<(ParaKey, Shaped)>>,
    pub(super) len: usize,
}

impl ParagraphStore {
    pub(super) fn new() -> Self {
        Self {
            buckets: HashMap::new(),
            len: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn get_mut(
        &mut self,
        hash: u64,
        matches: impl Fn(&ParaKey) -> bool,
    ) -> Option<&mut Shaped> {
        let bucket = self.buckets.get_mut(&hash)?;
        bucket
            .iter_mut()
            .find(|(key, _)| matches(key))
            .map(|(_, shaped)| shaped)
    }

    pub(super) fn insert(&mut self, key: ParaKey, shaped: Shaped) -> Option<Shaped> {
        let hash = hash_key(&key);
        let bucket = self.buckets.entry(hash).or_default();
        if let Some((_, slot)) = bucket.iter_mut().find(|(stored, _)| stored == &key) {
            return Some(std::mem::replace(slot, shaped));
        }
        bucket.push((key, shaped));
        self.len += 1;
        None
    }

    pub(super) fn drain_if(&mut self, mut drop_shaped: impl FnMut(&Shaped) -> bool) -> Vec<Shaped> {
        let mut dropped = Vec::new();
        self.buckets.retain(|_, bucket| {
            let mut index = 0;
            while index < bucket.len() {
                if drop_shaped(&bucket[index].1) {
                    let (_, shaped) = bucket.swap_remove(index);
                    self.len = self.len.saturating_sub(1);
                    dropped.push(shaped);
                } else {
                    index += 1;
                }
            }
            !bucket.is_empty()
        });
        dropped
    }

    pub(super) fn drain_all(&mut self) -> Vec<Shaped> {
        self.len = 0;
        let mut dropped = Vec::new();
        for (_, bucket) in self.buckets.drain() {
            dropped.extend(bucket.into_iter().map(|(_, shaped)| shaped));
        }
        dropped
    }
}

pub(super) fn hash_key(key: &ParaKey) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn hash_query(
    utf8: &str,
    font_size: u32,
    max_width: u32,
    color: [u8; 4],
    family: &str,
    max_lines: u32,
    ellipsis: &str,
    align: TextAlign,
    weight: u16,
    direction_rtl: bool,
    height_bits: u32,
    underline: bool,
    line_through: bool,
    soft_wrap: bool,
    overflow: protocol::TextOverflow,
    spans: &[TextSpan],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    utf8.hash(&mut hasher);
    font_size.hash(&mut hasher);
    max_width.hash(&mut hasher);
    color.hash(&mut hasher);
    family.hash(&mut hasher);
    max_lines.hash(&mut hasher);
    ellipsis.hash(&mut hasher);
    align.hash(&mut hasher);
    weight.hash(&mut hasher);
    direction_rtl.hash(&mut hasher);
    height_bits.hash(&mut hasher);
    underline.hash(&mut hasher);
    line_through.hash(&mut hasher);
    soft_wrap.hash(&mut hasher);
    overflow.hash(&mut hasher);
    spans.len().hash(&mut hasher);
    for span in spans {
        span.utf8.hash(&mut hasher);
        span.family.as_deref().unwrap_or("").hash(&mut hasher);
        span.weight.hash(&mut hasher);
        span.color.hash(&mut hasher);
        span.underline.hash(&mut hasher);
        span.line_through.hash(&mut hasher);
    }
    hasher.finish()
}

pub(super) fn key_matches(
    key: &ParaKey,
    utf8: &str,
    font_size: u32,
    max_width: u32,
    color: [u8; 4],
    family: &str,
    max_lines: u32,
    ellipsis: &str,
    align: TextAlign,
    weight: u16,
    direction_rtl: bool,
    height_bits: u32,
    underline: bool,
    line_through: bool,
    soft_wrap: bool,
    overflow: protocol::TextOverflow,
    spans: &[TextSpan],
) -> bool {
    key.utf8 == utf8
        && key.font_size == font_size
        && key.max_width == max_width
        && key.color == color
        && key.family == family
        && key.max_lines == max_lines
        && key.ellipsis == ellipsis
        && key.align == align
        && key.weight == weight
        && key.direction_rtl == direction_rtl
        && key.height_bits == height_bits
        && key.underline == underline
        && key.line_through == line_through
        && key.soft_wrap == soft_wrap
        && key.overflow == overflow
        && key.spans.len() == spans.len()
        && key.spans.iter().zip(spans).all(|(stored, span)| {
            stored.utf8 == span.utf8
                && stored.family == span.family.as_deref().unwrap_or("")
                && stored.weight == span.weight
                && stored.color == span.color
                && stored.underline == span.underline
                && stored.line_through == span.line_through
        })
}

pub(super) fn para_key(
    utf8: &str,
    font_size: f32,
    max_width: f32,
    color: [u8; 4],
    family: &str,
    style: &TextStyle,
    spans: &[TextSpan],
) -> Option<ParaKey> {
    if !font_size.is_finite() || font_size <= 0.0 || !max_width.is_finite() || max_width <= 0.0 {
        return None;
    }
    Some(ParaKey {
        utf8: utf8.to_owned(),
        font_size: font_size.to_bits(),
        max_width: max_width.to_bits(),
        color,
        family: family.to_owned(),
        max_lines: style.max_lines.unwrap_or(0),
        ellipsis: style.ellipsis.clone().unwrap_or_default(),
        align: style.align,
        weight: style.weight,
        direction_rtl: style.direction_rtl,
        height_bits: style.height.to_bits(),
        underline: style.underline,
        line_through: style.line_through,
        soft_wrap: style.soft_wrap,
        overflow: style.overflow,
        spans: spans
            .iter()
            .map(|span| SpanKey {
                utf8: span.utf8.clone(),
                family: span.family.clone().unwrap_or_default(),
                weight: span.weight,
                color: span.color,
                underline: span.underline,
                line_through: span.line_through,
            })
            .collect(),
    })
}

#[cfg(test)]
mod key_tests {
    use super::{hash_key, hash_query, key_matches, para_key};
    use protocol::{TextSpan, TextStyle};

    #[test]
    fn a_query_hashes_like_the_owned_key() {
        let style = TextStyle::default();
        let spans = [TextSpan {
            utf8: "ab".into(),
            color: [1, 2, 3, 4],
            weight: 700,
            underline: true,
            line_through: false,
            family: Some("Sans".into()),
        }];
        let key = para_key("ab", 16.0, 100.0, [9, 8, 7, 6], "Sans", &style, &spans).unwrap();
        let hash = hash_query(
            "ab",
            16.0f32.to_bits(),
            100.0f32.to_bits(),
            [9, 8, 7, 6],
            "Sans",
            style.max_lines.unwrap_or(0),
            style.ellipsis.as_deref().unwrap_or(""),
            style.align,
            style.weight,
            style.direction_rtl,
            style.height.to_bits(),
            style.underline,
            style.line_through,
            style.soft_wrap,
            style.overflow,
            &spans,
        );
        assert_eq!(hash, hash_key(&key));
        assert!(key_matches(
            &key,
            "ab",
            16.0f32.to_bits(),
            100.0f32.to_bits(),
            [9, 8, 7, 6],
            "Sans",
            style.max_lines.unwrap_or(0),
            style.ellipsis.as_deref().unwrap_or(""),
            style.align,
            style.weight,
            style.direction_rtl,
            style.height.to_bits(),
            style.underline,
            style.line_through,
            style.soft_wrap,
            style.overflow,
            &spans,
        ));
    }
}
