use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::DesktopError;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Bundle {
    pub schema_version: u32,
    pub artifact_bytes: u64,
    pub startup_bytes: u64,
    pub chunks: BTreeMap<String, Chunk>,
    pub pools: BTreeMap<String, Pool>,
    pub external_assets: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Chunk {
    rendered_javascript_bytes: u64,
    bytecode_payload_bytes: Option<u64>,
    pub modules: Vec<Module>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Module {
    pub path: String,
    pub group: String,
    pub rendered_bytes: u64,
    pub apportioned_bytecode_bytes: Option<u64>,
    pub retained_pool_bytes: u64,
    pub amortized_pool_bytes: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pool {
    pub dedup_saved_bytes: u64,
}

pub(crate) struct Item {
    pub label: String,
    pub path: Vec<String>,
    pub bytes: u64,
    pub color: u16,
    pub module: Option<Module>,
}

impl Bundle {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, DesktopError> {
        let bundle: Self = serde_json::from_slice(bytes)?;
        if bundle.schema_version != 1 {
            return Err(DesktopError::new("Unsupported bundle report schema"));
        }
        Ok(bundle)
    }

    pub(crate) fn children(&self, path: &[String], metric: &str) -> Vec<Item> {
        let metric_value = |rendered, bytecode: Option<u64>| {
            if metric == "bytecode" {
                bytecode.unwrap_or(rendered)
            } else {
                rendered
            }
        };
        let Some(name) = path.first() else {
            return self
                .chunks
                .iter()
                .map(|(name, chunk)| Item {
                    label: name.clone(),
                    path: vec![name.clone()],
                    bytes: metric_value(
                        chunk.rendered_javascript_bytes,
                        chunk.bytecode_payload_bytes,
                    ),
                    color: 64,
                    module: None,
                })
                .collect();
        };
        let Some(chunk) = self.chunks.get(name) else {
            return Vec::new();
        };
        let mut children = BTreeMap::<String, Item>::new();
        let prefix = &path[1..];
        for module in &chunk.modules {
            let components: Vec<_> = module
                .path
                .split('/')
                .filter(|part| !part.is_empty())
                .collect();
            if components.len() <= prefix.len()
                || !components
                    .iter()
                    .zip(prefix)
                    .all(|(part, prefix)| part == prefix)
            {
                continue;
            }
            let next = components[prefix.len()];
            let leaf = components.len() == prefix.len() + 1;
            let color = match module.group.as_str() {
                "application" => 64,
                "framework" => 65,
                "compiledAsset" => 67,
                "staticPool" => 68,
                _ => 66,
            };
            let key = format!("{next}{}", if leaf { "" } else { "/" });
            let item = children.entry(key.clone()).or_insert_with(|| {
                let mut path = path.to_vec();
                path.push(next.to_owned());
                Item {
                    label: key,
                    path,
                    bytes: 0,
                    color,
                    module: leaf.then(|| module.clone()),
                }
            });
            item.bytes = item.bytes.saturating_add(metric_value(
                module.rendered_bytes,
                module.apportioned_bytecode_bytes,
            ));
        }
        children.into_values().collect()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub(crate) fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

pub(crate) fn squarify(weights: &[u64], width: f64, height: f64) -> Vec<(usize, Rect)> {
    if width <= 0. || height <= 0. {
        return Vec::new();
    }
    let mut items: Vec<_> = weights
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, weight)| *weight != 0)
        .collect();
    items.sort_by_key(|&(index, weight)| (std::cmp::Reverse(weight), index));
    let total: f64 = items.iter().map(|(_, weight)| *weight as f64).sum();
    let area = |weight: u64| (weight as f64 / total) * width * height;
    let mut remaining = Rect {
        x: 0.,
        y: 0.,
        w: width,
        h: height,
    };
    let mut result = Vec::with_capacity(items.len());
    let mut index = 0;
    while index < items.len() {
        let short = remaining.w.min(remaining.h);
        if short <= 0. {
            break;
        }
        let start = index;
        let max = area(items[start].1);
        let mut sum = max;
        let score = |sum: f64, min: f64| {
            (short * short * max / sum.powi(2)).max(sum.powi(2) / (short * short * min))
        };
        let mut worst = score(sum, max);
        index += 1;
        while index < items.len() {
            let candidate = area(items[index].1);
            let next = score(sum + candidate, candidate);
            if next > worst {
                break;
            }
            worst = next;
            sum += candidate;
            index += 1;
        }
        let thickness = (sum / short).min(remaining.w.max(remaining.h));
        let vertical = remaining.w >= remaining.h;
        let mut offset: f64 = 0.;
        for &(original, weight) in &items[start..index] {
            let size = (area(weight) / thickness).min((short - offset).max(0.));
            let rect = if vertical {
                Rect {
                    x: remaining.x,
                    y: remaining.y + offset,
                    w: thickness,
                    h: size,
                }
            } else {
                Rect {
                    x: remaining.x + offset,
                    y: remaining.y,
                    w: size,
                    h: thickness,
                }
            };
            result.push((original, rect));
            offset += size;
        }
        if vertical {
            remaining.x += thickness;
            remaining.w -= thickness;
        } else {
            remaining.y += thickness;
            remaining.h -= thickness;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_container_in_proportion_to_weights() {
        let width = 503.;
        let height = 271.;
        let weights = [6, 2, 5, 1, 0, 5];
        let layout = squarify(&weights, width, height);
        assert_eq!(layout.len(), 5);
        for (index, rect) in layout {
            assert!(rect.x + rect.w <= width + 0.0001 && rect.y + rect.h <= height + 0.0001);
            assert!(
                (rect.w * rect.h / (width * height) - weights[index] as f64 / 19.).abs() < 0.0001
            );
        }
    }
}
