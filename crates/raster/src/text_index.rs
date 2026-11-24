//! UTF-8 byte offsets to UTF-16 code units and back.
//!
//! Parley indexes text in UTF-8 bytes; Impasto's editing state and line
//! boxes use UTF-16 code units. Pure ASCII needs no table: both offsets
//! are the same number.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Utf8ToUtf16 {
    /// Byte offset == unit offset.
    Ascii(usize),
    /// Unit offset at each char boundary, plus one past the end.
    Table {
        /// UTF-8 byte offset of each entry in `units`.
        bytes: Box<[usize]>,
        units: Box<[u32]>,
        len: usize,
    },
}

impl Utf8ToUtf16 {
    pub fn new(text: &str) -> Self {
        if text.is_ascii() {
            return Self::Ascii(text.len());
        }
        let mut bytes = Vec::with_capacity(text.len() + 1);
        let mut units = Vec::with_capacity(text.len() + 1);
        let mut unit = 0u32;
        for (index, ch) in text.char_indices() {
            bytes.push(index);
            units.push(unit);
            unit = unit.saturating_add(if ch.len_utf16() == 2 { 2 } else { 1 });
        }
        bytes.push(text.len());
        units.push(unit);
        Self::Table {
            bytes: bytes.into(),
            units: units.into(),
            len: text.len(),
        }
    }

    /// Total UTF-16 units.
    #[cfg(test)]
    pub fn units(&self) -> u32 {
        match self {
            Self::Ascii(len) => u32::try_from(*len).unwrap_or(u32::MAX),
            Self::Table { units, .. } => *units.last().unwrap_or(&0),
        }
    }

    /// Byte offset of a UTF-16 unit. Out of range clamps to the end.
    pub fn byte_at(&self, unit: u32) -> usize {
        match self {
            Self::Ascii(len) => (unit as usize).min(*len),
            Self::Table { bytes, units, len } => {
                let index = units.partition_point(|&u| u < unit);
                bytes.get(index).copied().unwrap_or(*len)
            }
        }
    }

    /// UTF-16 unit of a byte offset. A byte inside a scalar maps to the
    /// unit of that scalar's start.
    pub fn unit_at(&self, byte: usize) -> u32 {
        match self {
            Self::Ascii(len) => u32::try_from(byte.min(*len)).unwrap_or(u32::MAX),
            Self::Table { bytes, units, len } => {
                if byte >= *len {
                    return *units.last().unwrap_or(&0);
                }
                let index = bytes.partition_point(|&b| b <= byte);
                units.get(index.saturating_sub(1)).copied().unwrap_or(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8ToUtf16;

    #[test]
    fn ascii_is_identity() {
        let map = Utf8ToUtf16::new("Hello");
        assert_eq!(map.units(), 5);
        assert_eq!(map.byte_at(3), 3);
        assert_eq!(map.unit_at(5), 5);
        assert_eq!(map.byte_at(99), 5);
    }

    #[test]
    fn surrogates_combining_and_cjk_count_units() {
        // A (1 unit) + combining mark (1) + 中 (1) + emoji (2 units)
        let text = "A\u{301}\u{4E2D}\u{1F600}";
        let map = Utf8ToUtf16::new(text);
        assert_eq!(map.units(), 5);
        assert_eq!(map.unit_at(0), 0);
        assert_eq!(map.unit_at(1), 1); // combining mark
        assert_eq!(map.unit_at(3), 2); // 中
        assert_eq!(map.unit_at(6), 3); // emoji start
        assert_eq!(map.unit_at(text.len()), 5);
        assert_eq!(map.byte_at(0), 0);
        assert_eq!(map.byte_at(1), 1);
        assert_eq!(map.byte_at(2), 3);
        assert_eq!(map.byte_at(3), 6);
        // Inside the surrogate pair maps to the pair's end.
        assert_eq!(map.byte_at(4), text.len());
        assert_eq!(map.byte_at(5), text.len());
        assert_eq!(map.byte_at(6), text.len());
    }
}
