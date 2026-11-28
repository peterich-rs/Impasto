use std::collections::BTreeMap;

use devtools_symbols::Resolver;
use serde_json::Value;

use crate::types::{Row, RowSlice, clip, matches};

#[derive(Default)]
pub(crate) struct Tree {
    nodes: Vec<Node>,
    identities: BTreeMap<u32, usize>,
}

#[derive(Default)]
struct Node {
    symbol: String,
    children: BTreeMap<String, usize>,
    total: u32,
    own: u32,
    depth: u32,
    identity: u32,
}

pub(crate) struct Block<'a> {
    pub id: u32,
    pub symbol: &'a str,
    pub start: f64,
    pub end: f64,
    pub depth: u32,
    pub total: u32,
    pub own: u32,
}

impl Tree {
    pub(crate) fn build<'a>(
        samples: impl Iterator<Item = &'a Value>,
        symbols: Option<&Resolver<'_>>,
        frame: Option<u64>,
    ) -> Self {
        let mut tree = Self {
            nodes: vec![Node::default()],
            identities: BTreeMap::new(),
        };
        for sample in samples {
            let sample = &sample["value"];
            if frame.is_some_and(|frame| sample["frame_id"].as_u64() != Some(frame)) {
                continue;
            }
            let Some(raw) = sample["stack"].as_str() else {
                continue;
            };
            let stack = symbols.map_or_else(|| raw.to_owned(), |resolver| resolver.remap(raw));
            let frames: Vec<_> = stack
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with("at "))
                .take(64)
                .collect();
            if frames.is_empty() {
                continue;
            }
            let mut node = 0;
            tree.nodes[node].total += 1;
            for frame in frames.into_iter().rev() {
                let frame = clip(frame.trim_start_matches("at "), 512);
                let next = if let Some(child) = tree.nodes[node].children.get(&frame) {
                    *child
                } else {
                    if tree.nodes.len() >= 65_536 {
                        break;
                    }
                    let id = tree.nodes.len();
                    let depth = tree.nodes[node].depth + 1;
                    let mut identity = frame
                        .bytes()
                        .fold(tree.nodes[node].identity ^ 2_166_136_261, |hash, byte| {
                            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
                        });
                    while identity == 0 || tree.identities.contains_key(&identity) {
                        identity = identity.wrapping_add(1);
                    }
                    tree.identities.insert(identity, id);
                    tree.nodes.push(Node {
                        identity,
                        symbol: frame.clone(),
                        depth,
                        ..Node::default()
                    });
                    tree.nodes[node].children.insert(frame, id);
                    id
                };
                node = next;
                tree.nodes[node].total += 1;
            }
            tree.nodes[node].own += 1;
        }
        tree
    }

    pub(crate) fn blocks(&self, identity: u32, min_ratio: f64) -> Vec<Block<'_>> {
        let zoom = self.identities.get(&identity).copied().unwrap_or(0);
        let Some(root) = self.nodes.get(zoom).filter(|entry| entry.total > 0) else {
            return Vec::new();
        };
        let total = f64::from(root.total);
        let mut result = Vec::new();
        let mut pending = vec![(zoom, 0., 1.)];
        while let Some((id, start, end)) = pending.pop() {
            let node = &self.nodes[id];
            if end - start < min_ratio {
                continue;
            }
            if id != 0 {
                result.push(Block {
                    id: node.identity,
                    symbol: &node.symbol,
                    start,
                    end,
                    depth: node.depth - root.depth,
                    total: node.total,
                    own: node.own,
                });
            }
            let mut x = start;
            for &child in node.children.values() {
                let next = (x + f64::from(self.nodes[child].total) / total).min(end);
                pending.push((child, x, next));
                x = next;
            }
        }
        result
    }

    pub(crate) fn table(&self, mode: &str, search: &str, offset: u32, limit: u32) -> RowSlice {
        let query = search.to_lowercase();
        if mode == "bottom_up" {
            let mut symbols = BTreeMap::<_, (u32, u32)>::new();
            for node in self
                .nodes
                .iter()
                .skip(1)
                .filter(|node| matches(&node.symbol, &query))
            {
                let (own, total) = symbols.entry(node.symbol.as_str()).or_default();
                *own += node.own;
                *total += node.total;
            }
            let mut rows: Vec<_> = symbols.into_iter().collect();
            rows.sort_by_key(|(symbol, (own, total))| {
                (std::cmp::Reverse(*own), std::cmp::Reverse(*total), *symbol)
            });
            return RowSlice::page(rows, offset, limit, |(name, (own, total))| {
                row(0, name, 0, total, own)
            });
        }
        let mut rows = Vec::new();
        let mut pending = vec![0];
        while let Some(id) = pending.pop() {
            let node = &self.nodes[id];
            if id != 0 && matches(&node.symbol, &query) {
                rows.push((id, node));
            }
            let mut children: Vec<_> = node.children.values().copied().collect();
            children.sort_by_key(|&id| self.nodes[id].total);
            pending.extend(children);
        }
        RowSlice::page(rows, offset, limit, |(id, node)| {
            row(
                id,
                &node.symbol,
                node.depth.saturating_sub(1),
                node.total,
                node.own,
            )
        })
    }
}

fn row(id: usize, name: &str, depth: u32, total: u32, own: u32) -> Row {
    Row {
        id: id.to_string(),
        name: name.to_owned(),
        detail: format!("{own} self · {total} total"),
        depth,
        count: f64::from(total),
        ..Row::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_prefix_and_normalizes_zoom() {
        let samples = [
            json!({"value": {"stack": "    at a (app.js:1:1)\n    at root (app.js:3:1)", "frame_id": 1}}),
            json!({"value": {"stack": "    at b (app.js:2:1)\n    at root (app.js:3:1)", "frame_id": 1}}),
            json!({"value": {"stack": "    at unrelated", "frame_id": 2}}),
        ];
        let tree = Tree::build(samples.iter(), None, Some(1));
        let items = tree.blocks(tree.nodes[1].identity, 0.);
        let prefix = &items[0];
        assert_eq!(
            (prefix.total, prefix.own, prefix.start, prefix.end),
            (2, 0, 0., 1.)
        );
        assert_eq!(items.len(), 3);
    }
}
