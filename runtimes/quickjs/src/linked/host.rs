//! Header helpers for the QuickJS host globals.

pub(super) const PRELUDE: &str = include_str!("../prelude.js");

pub(super) fn header_pairs(block: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in block.split('\n') {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        pairs.push((name.to_owned(), value.trim().to_owned()));
    }
    pairs
}

pub(super) fn header_block(headers: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, value) in headers {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
    }
    out
}

pub(super) fn app_state(name: &str) -> &'static str {
    match name {
        "resumed" => "active",
        "inactive" => "inactive",
        "hidden" | "paused" | "detached" => "background",
        _ => "unknown",
    }
}
