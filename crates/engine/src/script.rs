//! Split a paragraph into Latin and CJK runs.
//!
//! Characters with no script of their own stay with the previous run, so
//! punctuation does not change family by itself.

/// `true` means the run should use the CJK family when one is registered.
pub fn split_scripts(text: &str) -> Vec<(&str, bool)> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = false;
    let mut started = false;
    for (byte, ch) in text.char_indices() {
        let next = script_of(ch).unwrap_or(current);
        if started && next != current {
            runs.push((&text[start..byte], current));
            start = byte;
        }
        current = next;
        started = true;
    }
    if started && start < text.len() {
        runs.push((&text[start..], current));
    }
    runs
}

fn script_of(ch: char) -> Option<bool> {
    let code = ch as u32;
    if is_cjk(code) {
        Some(true)
    } else if ch.is_alphanumeric() {
        Some(false)
    } else {
        None
    }
}

fn is_cjk(code: u32) -> bool {
    matches!(
        code,
        0x1100..=0x11FF
            | 0x2E80..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0xFF00..=0xFFEF
            | 0x20000..=0x3FFFF
    )
}

#[cfg(test)]
mod tests {
    use super::split_scripts;

    #[test]
    fn han_is_its_own_run_and_punctuation_stays() {
        let runs = split_scripts("Hi 你好!");
        assert_eq!(runs, vec![("Hi ", false), ("你好!", true)]);
    }
}
