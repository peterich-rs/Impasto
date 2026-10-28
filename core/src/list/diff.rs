//! Render-op diff. The language binding owns slot pools and applies these ops.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShownRow {
    pub key: String,
    pub ty: String,
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListOp {
    Detach {
        key: String,
    },
    Adopt {
        key: String,
        ty: String,
        index: u32,
    },
    Set {
        key: String,
        index: u32,
    },
    /// `before` empty means the row is the last child.
    Place {
        key: String,
        before: String,
    },
}

/// Detach rows that left or changed type, adopt or set the rest, then place
/// from the end so each anchor is already in the parent.
pub fn diff_rows(previous: &[ShownRow], next: &[ShownRow], force_set: bool) -> Vec<ListOp> {
    let mut next_of: HashMap<&str, &ShownRow> = HashMap::new();
    let mut order = Vec::new();
    for row in next {
        if next_of.contains_key(row.key.as_str()) {
            continue;
        }
        next_of.insert(row.key.as_str(), row);
        order.push(row.key.as_str());
    }
    let mut prev_of: HashMap<&str, &ShownRow> = HashMap::new();
    for row in previous {
        prev_of.entry(row.key.as_str()).or_insert(row);
    }

    let mut ops = Vec::new();
    for row in previous {
        match next_of.get(row.key.as_str()) {
            Some(wanted) if wanted.ty == row.ty => {}
            _ => ops.push(ListOp::Detach {
                key: row.key.clone(),
            }),
        }
    }

    let mut live = Vec::new();
    for key in &order {
        let Some(wanted) = next_of.get(key) else {
            continue;
        };
        match prev_of.get(key) {
            Some(prev) if prev.ty == wanted.ty => {
                if force_set || prev.index != wanted.index {
                    ops.push(ListOp::Set {
                        key: (*key).to_owned(),
                        index: wanted.index,
                    });
                }
            }
            _ => ops.push(ListOp::Adopt {
                key: (*key).to_owned(),
                ty: wanted.ty.clone(),
                index: wanted.index,
            }),
        }
        live.push(*key);
    }

    let same_order = previous.len() == live.len()
        && previous
            .iter()
            .zip(live.iter())
            .all(|(row, key)| row.key == *key && next_of.get(*key).is_some_and(|n| n.ty == row.ty));
    if !same_order {
        for index in (0..live.len()).rev() {
            let before = if index + 1 < live.len() {
                live[index + 1].to_owned()
            } else {
                String::new()
            };
            ops.push(ListOp::Place {
                key: live[index].to_owned(),
                before,
            });
        }
    }
    ops
}
