//! Intent data URI. Missing or empty is not a launch URL.

pub(crate) fn from_intent(data: Option<&str>) -> Option<String> {
    let data = data?.trim();
    if data.is_empty() {
        None
    } else {
        Some(data.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::from_intent;

    #[test]
    fn skips_missing_and_empty() {
        assert_eq!(from_intent(None), None);
        assert_eq!(from_intent(Some("")), None);
        assert_eq!(from_intent(Some("  ")), None);
        assert_eq!(
            from_intent(Some("https://app.example/item")),
            Some("https://app.example/item".into())
        );
        assert_eq!(
            from_intent(Some(" file:///tmp/a ")),
            Some("file:///tmp/a".into())
        );
    }
}
