use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Recording;
use crate::types::{Row, RowSlice, clip, id, matches, number, string};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct ViewIndex {
    nodes: Vec<Node>,
    #[serde(skip)]
    indices: BTreeMap<String, usize>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Node {
    id: String,
    value: Value,
    depth: u32,
    end: usize,
    parent: Option<usize>,
}

impl ViewIndex {
    pub(crate) fn new(mut snapshot: Value) -> Self {
        let mut result = Self::default();
        result.insert(snapshot["root"].take(), 0, None);
        result
    }

    fn insert(&mut self, mut value: Value, depth: u32, parent: Option<usize>) {
        let key = id(&value["id"]);
        if key.is_empty() || self.nodes.len() >= 20_000 || depth >= 128 {
            return;
        }
        let children = value
            .as_object_mut()
            .and_then(|node| node.remove("children"));
        let index = self.nodes.len();
        self.indices.insert(key.clone(), index);
        self.nodes.push(Node {
            id: key,
            value,
            depth,
            end: 0,
            parent,
        });
        if let Some(Value::Array(children)) = children {
            for child in children {
                self.insert(child, depth + 1, Some(index));
            }
        }
        self.nodes[index].end = self.nodes.len();
    }

    pub(crate) fn reindex(&mut self) {
        self.nodes.truncate(20_000);
        let length = self.nodes.len();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            node.end = node.end.clamp(index + 1, length);
            node.parent = node.parent.filter(|parent| *parent < index);
            node.depth = node.depth.min(128);
        }
        self.indices = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect();
    }

    pub(crate) fn detail(&self, key: &str) -> Option<Value> {
        self.indices
            .get(key)
            .and_then(|index| self.nodes.get(*index))
            .map(|entry| entry.value.clone())
    }

    pub(crate) fn ancestors(&self, id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = self
            .indices
            .get(id)
            .and_then(|index| self.nodes.get(*index))
            .and_then(|node| node.parent);
        while let Some(index) = current {
            let Some(node) = self.nodes.get(index) else {
                break;
            };
            result.push(node.id.clone());
            current = node.parent.filter(|parent| *parent < index);
        }
        result
    }

    pub(crate) fn position(&self, key: &str, expanded: Vec<String>) -> Option<u32> {
        let selected = *self.indices.get(key)?;
        let expanded: HashSet<_> = expanded.into_iter().collect();
        let mut index = 0;
        let mut position = 0;
        while index < selected {
            let node = self.nodes.get(index)?;
            let next = if node.depth == 0 || expanded.contains(&node.id) {
                index + 1
            } else {
                node.end.max(index + 1)
            };
            index = next;
            position += 1;
        }
        (index == selected).then_some(position)
    }

    pub(crate) fn rows(
        &self,
        data: &Recording,
        expanded: Vec<String>,
        query: &str,
        path: &str,
        offset: u32,
        limit: u32,
    ) -> RowSlice {
        let query = query.to_lowercase();
        let expanded: HashSet<_> = expanded.into_iter().collect();
        let heat = recent_repaints(data);
        let mut index = 0;
        let visible = std::iter::from_fn(|| {
            while let Some(node) = self.nodes.get(index) {
                let expandable = node.end > index + 1;
                if !expandable
                    || expanded.contains(&node.id)
                    || !query.is_empty()
                    || !path.is_empty()
                    || node.depth == 0
                {
                    index += 1;
                } else {
                    index = node.end;
                }
                let source = &node.value["source"];
                let heat = heat.get(node.id.as_str());
                if (path.is_empty() || heat.is_some_and(|entry| entry.paths.contains(path)))
                    && [
                        node.id.as_str(),
                        string(&node.value, "kind"),
                        string(&node.value, "text"),
                        string(source, "component"),
                        string(source, "path"),
                    ]
                    .iter()
                    .any(|text| matches(text, &query))
                {
                    return Some((node, expandable, heat));
                }
            }
            None
        });
        RowSlice::page(visible, offset, limit, |(node, expandable, heat)| {
            let kind = string(&node.value, "kind");
            let source = &node.value["source"];
            let component = string(source, "component");
            Row {
                id: node.id.clone(),
                kind: kind.to_owned(),
                name: clip(
                    if component.is_empty() {
                        kind
                    } else {
                        component
                    },
                    96,
                ),
                detail: clip(string(&node.value, "text"), 128),
                meta: heat
                    .map(|heat| format!("{} / 60 frames", heat.count))
                    .unwrap_or_default(),
                depth: node.depth,
                expandable,
                count: number(&node.value, "totalRepaints"),
                ..Row::default()
            }
        })
    }
}

#[derive(Default)]
pub(crate) struct RepaintHeat<'a> {
    pub count: u32,
    pub paths: HashSet<&'a str>,
}

pub(crate) fn recent_repaints(data: &Recording) -> BTreeMap<&str, RepaintHeat<'_>> {
    let mut heat = BTreeMap::<_, RepaintHeat>::new();
    let latest = data
        .frames
        .entries()
        .back()
        .map(|frame| frame.id)
        .or_else(|| data.repaints.last_key_value().map(|(id, _)| *id))
        .unwrap_or_default();
    for (_, batch) in data.repaints.range(latest.saturating_sub(59)..=latest) {
        for entry in batch["entries"].as_array().into_iter().flatten() {
            let node = string(entry, "node_raw");
            let entry_heat = heat.entry(node).or_default();
            entry_heat.count += 1;
            entry_heat.paths.insert(string(entry, "path"));
        }
    }
    heat
}

pub(crate) fn repaints(
    data: &Recording,
    frame: u64,
    path: &str,
    sort: &str,
    offset: u32,
    limit: u32,
) -> RowSlice {
    let mut entries: Vec<_> = data
        .repaints
        .get(&frame)
        .and_then(|batch| batch["entries"].as_array())
        .into_iter()
        .flatten()
        .filter(|entry| path.is_empty() || entry["path"] == path)
        .collect();
    if sort == "commands" {
        entries.sort_by_key(|entry| std::cmp::Reverse(entry["command_count"].as_u64()));
    }
    RowSlice::page(entries, offset, limit, |entry| {
        let component = string(&entry["source"], "component");
        let kind = string(entry, "kind");
        Row {
            id: id(&entry["node_raw"]),
            kind: string(entry, "path").to_owned(),
            name: if component.is_empty() {
                kind
            } else {
                component
            }
            .to_owned(),
            detail: string(&entry["source"], "path").to_owned(),
            count: number(entry, "command_count"),
            ..Row::default()
        }
    })
}
