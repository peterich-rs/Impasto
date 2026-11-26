//! Versioned wire format for declarations published by the TSX compiler.

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use serde::{Serialize, de::DeserializeOwned};

use super::{Declaration, Style};

const VERSION: &str = "1:";
const MAX_ENCODED_BYTES: usize = 256 * 1024;

/// Encode declarations after property validation and shorthand expansion.
pub fn encode_style(style: &Style) -> Result<String, String> {
    encode(style.fragments())
}

/// Decode compiled declarations without re-parsing author styles.
pub fn decode_style(blob: &str) -> Result<Style, String> {
    let fragments: Vec<Vec<Declaration>> = decode(blob)?;
    let mut style = Style::new();
    for fragment in fragments {
        style
            .push_fragment(fragment)
            .map_err(|error| error.to_string())?;
    }
    Ok(style)
}

pub(crate) fn encode<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    encode_version(value, VERSION)
}

pub(crate) fn encode_version<T: Serialize + ?Sized>(
    value: &T,
    version: &str,
) -> Result<String, String> {
    let bytes = postcard::to_allocvec(value).map_err(|error| error.to_string())?;
    let blob = format!("{version}{}", STANDARD_NO_PAD.encode(bytes));
    if blob.len() > MAX_ENCODED_BYTES {
        return Err("compiled style exceeds the pool entry size limit".into());
    }
    Ok(blob)
}

pub(crate) fn decode<T: DeserializeOwned>(blob: &str) -> Result<T, String> {
    decode_version(blob, VERSION)
}

pub(crate) fn decode_version<T: DeserializeOwned>(blob: &str, version: &str) -> Result<T, String> {
    if blob.len() > MAX_ENCODED_BYTES {
        return Err("compiled style exceeds the pool entry size limit".into());
    }
    let blob = blob
        .strip_prefix(version)
        .ok_or_else(|| "unknown compiled style format".to_owned())?;
    let bytes = STANDARD_NO_PAD
        .decode(blob)
        .map_err(|error| error.to_string())?;
    let (value, rest) = postcard::take_from_bytes(&bytes).map_err(|error| error.to_string())?;
    if !rest.is_empty() {
        return Err("trailing data in compiled style".into());
    }
    Ok(value)
}
