use std::collections::HashSet;

use serde_json::Value;

use crate::types::{Row, RowSlice, clip, matches};

pub(crate) struct Index(Vec<Node>);

struct Node {
    key: String,
    kind: &'static str,
    preview: String,
    depth: u32,
    end: usize,
}

impl Index {
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let value = serde_json::from_slice(bytes).ok()?;
        let mut index = Self(Vec::new());
        index.push("$".to_owned(), value, 0);
        Some(index)
    }

    pub(crate) fn memory_cost(&self) -> usize {
        self.0.capacity() * size_of::<Node>()
            + self
                .0
                .iter()
                .map(|node| node.key.capacity() + node.preview.capacity())
                .sum::<usize>()
    }

    fn push(&mut self, key: String, value: Value, depth: u32) {
        const MAX_NODES: usize = 125_000;
        if self.0.len() >= MAX_NODES {
            return;
        }
        let (kind, preview) = match &value {
            Value::Object(map) => ("object", format!("{} fields", map.len())),
            Value::Array(list) => ("array", format!("{} items", list.len())),
            Value::String(text) => ("string", clip(text, 240)),
            Value::Null => ("null", "null".to_owned()),
            Value::Number(_) => ("number", value.to_string()),
            Value::Bool(_) => ("boolean", value.to_string()),
        };
        let start = self.0.len();
        self.0.push(Node {
            key: clip(&key, 200),
            kind,
            preview,
            depth,
            end: start + 1,
        });
        match value {
            Value::Object(values) => {
                for (key, child) in values {
                    self.push(key, child, depth + 1);
                }
            }
            Value::Array(values) => {
                for (index, child) in values.into_iter().enumerate() {
                    self.push(index.to_string(), child, depth + 1);
                }
            }
            _ => {}
        }
        self.0[start].end = self.0.len();
    }

    pub(crate) fn slice(
        &self,
        expanded: Vec<u32>,
        search: &str,
        offset: u32,
        limit: u32,
    ) -> RowSlice {
        let expanded: HashSet<_> = expanded.into_iter().collect();
        let search = search.to_lowercase();
        let mut index = 0;
        let iter = std::iter::from_fn(|| {
            while let Some(node) = self.0.get(index) {
                let current = index;
                let expandable = node.end > index + 1;
                if !expandable
                    || index == 0
                    || !search.is_empty()
                    || u32::try_from(index).is_ok_and(|index| expanded.contains(&index))
                {
                    index += 1;
                } else {
                    index = node.end;
                }
                if matches(&node.key, &search) || matches(&node.preview, &search) {
                    return Some((current, node, expandable));
                }
            }
            None
        });
        RowSlice::page(iter, offset, limit, |(index, node, expandable)| Row {
            id: index.to_string(),
            name: node.key.clone(),
            kind: node.kind.to_owned(),
            detail: node.preview.clone(),
            depth: node.depth,
            expandable,
            ..Row::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_collapsed_subtrees_and_searches_hidden_values() {
        let index = Index::parse(br#"{"a":[{"key":"needle"}],"b":2}"#).expect("fixture");
        let closed = index.slice(Vec::new(), "", 0, 20);
        assert_eq!(closed.total, 3);
        let expanded = index.slice(vec![1, 2], "", 2, 2);
        assert_eq!(expanded.total, 5);
        assert_eq!(expanded.rows[1].name, "key");
        let search = index.slice(Vec::new(), "NEEDLE", 0, 1);
        assert_eq!(search.rows[0].name, "key");
    }
}
