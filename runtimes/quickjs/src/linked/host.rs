//! JSON and header helpers for the QuickJS host globals.

use session::PluginValue;

pub(super) const PRELUDE: &str = include_str!("../prelude.js");

pub(super) fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(super) fn encode(value: &PluginValue) -> String {
    match value {
        PluginValue::Null => "null".to_owned(),
        PluginValue::Bool(true) => "true".to_owned(),
        PluginValue::Bool(false) => "false".to_owned(),
        PluginValue::F64(number) if number.is_finite() => {
            let text = number.to_string();
            if text == "-0" { "0".to_owned() } else { text }
        }
        PluginValue::F64(_) => "null".to_owned(),
        PluginValue::Str(text) => json_string(text),
        PluginValue::Bytes(_) => "null".to_owned(),
        PluginValue::List(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&encode(item));
            }
            out.push(']');
            out
        }
        PluginValue::Map(entries) => {
            let mut out = String::from("{");
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                out.push_str(&encode(item));
            }
            out.push('}');
            out
        }
    }
}

pub(super) fn parse_args(json: &str) -> Result<Vec<PluginValue>, String> {
    let mut parser = Parser {
        text: json,
        index: 0,
    };
    parser.skip();
    if parser.done() {
        return Ok(Vec::new());
    }
    parser.array()
}

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

struct Parser<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn done(&self) -> bool {
        self.index >= self.text.len()
    }

    fn skip(&mut self) {
        while let Some(ch) = self.text[self.index..].chars().next() {
            if ch.is_ascii_whitespace() {
                self.index += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn array(&mut self) -> Result<Vec<PluginValue>, String> {
        self.skip();
        if self.bump() != Some('[') {
            return Err("module arguments".to_owned());
        }
        let mut items = Vec::new();
        loop {
            self.skip();
            if self.peek() == Some(']') {
                self.bump();
                break;
            }
            if !items.is_empty() {
                if self.bump() != Some(',') {
                    return Err("module arguments".to_owned());
                }
                self.skip();
            }
            items.push(self.value()?);
        }
        Ok(items)
    }

    fn value(&mut self) -> Result<PluginValue, String> {
        self.skip();
        match self.peek() {
            Some('n') => self.word("null").map(|()| PluginValue::Null),
            Some('t') => self.word("true").map(|()| PluginValue::Bool(true)),
            Some('f') => self.word("false").map(|()| PluginValue::Bool(false)),
            Some('"') => self.string().map(PluginValue::Str),
            Some('-') | Some('0'..='9') => self.number(),
            _ => Err("module arguments".to_owned()),
        }
    }

    fn word(&mut self, expected: &str) -> Result<(), String> {
        if self.text[self.index..].starts_with(expected) {
            self.index += expected.len();
            Ok(())
        } else {
            Err("module arguments".to_owned())
        }
    }

    fn number(&mut self) -> Result<PluginValue, String> {
        let start = self.index;
        if self.peek() == Some('-') {
            self.bump();
        }
        while matches!(self.peek(), Some('0'..='9' | '.')) {
            self.bump();
        }
        let text = &self.text[start..self.index];
        let number = text
            .parse::<f64>()
            .map_err(|_| "module arguments".to_owned())?;
        if !number.is_finite() {
            return Err("module arguments".to_owned());
        }
        Ok(PluginValue::F64(number))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.bump() != Some('"') {
            return Err("module arguments".to_owned());
        }
        let mut out = String::new();
        while let Some(ch) = self.bump() {
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self.bump().ok_or_else(|| "module arguments".to_owned())?;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                hex.push(self.bump().ok_or_else(|| "module arguments".to_owned())?);
                            }
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| "module arguments".to_owned())?;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("module arguments".to_owned()),
                    }
                }
                other => out.push(other),
            }
        }
        Err("module arguments".to_owned())
    }
}
