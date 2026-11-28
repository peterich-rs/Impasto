use impasto::ImpastoType;

use crate::store::{Recording, frames, network, views};
use crate::types::{id, number, string};

#[derive(Clone, PartialEq, ImpastoType)]
pub struct Finding {
    pub id: String,
    pub severity: String,
    pub title: String,
    pub metric: String,
    pub summary: String,
    pub page: String,
    pub selection: String,
    pub tab: String,
}

pub(crate) fn evaluate(data: &Recording) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut add = |id: &str,
                   severity: &str,
                   title: &str,
                   metric: String,
                   summary: &str,
                   page: &str,
                   selection: String,
                   tab: &str| {
        findings.push(Finding {
            id: id.to_owned(),
            severity: severity.to_owned(),
            title: title.to_owned(),
            metric,
            summary: summary.to_owned(),
            page: page.to_owned(),
            selection,
            tab: tab.to_owned(),
        });
    };
    let recent = || data.frames.entries().iter().rev().take(60);
    let jank = recent().filter(|frame| frame.over_budget).count();
    let slowest = recent().max_by(|a, b| frames::duration(a).total_cmp(&frames::duration(b)));
    let most_over_budget = recent()
        .filter(|frame| frame.budget_ms > 0. && frame.e2e_ms > 2. * frame.budget_ms)
        .max_by(|a, b| a.e2e_ms.total_cmp(&b.e2e_ms));
    if let Some(frame) = if jank >= 3 { slowest } else { most_over_budget } {
        add(
            "frame.jank_burst",
            "critical",
            "Frames exceeded their budget",
            format!("{jank} / 60 · worst {:.2} ms", frames::duration(frame)),
            "Inspect the slowest frame to find the stage that consumed its budget.",
            "frame",
            frame.id.to_string(),
            "breakdown",
        );
    }
    let blocked = recent()
        .take_while(|frame| frame.compositor_block.is_some())
        .count();
    if blocked >= 5
        && let Some(block) = recent()
            .next()
            .and_then(|frame| frame.compositor_block.as_ref())
    {
        add(
            "frame.compositor_block",
            "warning",
            "Compositor fast path blocked",
            format!("{blocked} consecutive frames · {}", block.reason),
            "Inspect the node that forced full UI work.",
            "view",
            block.node.clone().unwrap_or_default(),
            "tree",
        );
    }
    if let Some((node, heat)) = views::recent_repaints(data)
        .into_iter()
        .filter(|(_, entry)| {
            entry.count >= 30
                && (entry.paths.contains("full_picture_rebuild")
                    || entry.paths.contains("canvas_redraw"))
        })
        .max_by_key(|(_, entry)| entry.count)
    {
        add(
            "view.repaint_hotspot",
            "warning",
            "Repeated expensive repaint",
            format!("{} / 60 frames", heat.count),
            "Inspect the repaint boundary and the reactive reads that invalidate it.",
            "view",
            node.to_owned(),
            "tree",
        );
    }
    if let Some(issue) = data.layouts["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .max_by(|a, b| number(a, "excess").total_cmp(&number(b, "excess")))
    {
        add(
            "view.layout_overflow",
            "warning",
            "Layout overflows its container",
            format!(
                "{:.1} px on {}",
                number(issue, "excess"),
                string(issue, "axis")
            ),
            "Inspect the parent constraints and its largest overflowing child.",
            "view",
            id(&issue["parent"]),
            "layout",
        );
    }
    let recent_network = || data.network.values().rev().take(50);
    let total = recent_network().count();
    let errors = recent_network()
        .filter(|record| network::failed(record))
        .count();
    if errors >= 3 && errors * 100 >= total * 15 {
        add(
            "net.error_rate",
            "warning",
            "High request error rate",
            format!("{errors} / {total} requests"),
            "Filter the most recent request failures and inspect their status.",
            "network",
            String::new(),
            "error",
        );
    }
    if let Some(request) = data
        .network
        .values()
        .filter(|record| record["kind"] == "http" && record["state"] == "completed")
        .filter(|record| {
            number(&record["timing"], "waiting_ttfb_ms") > 800. || network::elapsed(record) > 3000.
        })
        .max_by(|a, b| network::elapsed(a).total_cmp(&network::elapsed(b)))
    {
        add(
            "net.slow_ttfb",
            "info",
            "Slow completed request",
            format!(
                "{:.0} ms TTFB · {:.0} ms total",
                number(&request["timing"], "waiting_ttfb_ms"),
                network::elapsed(request)
            ),
            "Inspect the request timing and redirect hops.",
            "network",
            id(&request["token"]),
            "timing",
        );
    }
    let heap = data
        .runtime
        .iter()
        .rev()
        .take(10)
        .map(|value| number(&value["value"]["detail"], "memory_used_size"))
        .collect::<Vec<_>>();
    let growth = heap
        .first()
        .zip(heap.last())
        .map_or(0., |(last, first)| last - first);
    let async_details = data
        .resources
        .iter()
        .rev()
        .filter(|entry| entry["value"]["kind"] == "async")
        .take(10)
        .map(|entry| &entry["value"]["detail"])
        .collect::<Vec<_>>();
    let timers = async_details
        .first()
        .zip(async_details.last())
        .is_some_and(|(last, first)| {
            number(&last["counts"], "timers") - number(&first["counts"], "timers") >= 32.
                || number(last, "job_limit_hits") - number(first, "job_limit_hits") >= 10.
        });
    if heap.len() == 10
        && growth > 8. * 1024. * 1024.
        && heap.windows(2).all(|pair| pair[0] >= pair[1])
        || async_details.len() == 10 && timers
    {
        add(
            "js.memory_or_timer_growth",
            "warning",
            "JS resources are growing",
            if timers {
                "Timers or job limits increased".to_owned()
            } else {
                format!("+{:.1} MiB", growth / 1048576.)
            },
            "Compare the heap before and after GC and inspect retained async work.",
            "js",
            String::new(),
            "resources",
        );
    }
    findings.sort_by_key(|finding| match finding.severity.as_str() {
        "critical" => 0,
        "warning" => 1,
        _ => 2,
    });
    findings
}

pub(crate) fn evidence(data: &Recording, finding: &str) -> Vec<crate::types::Row> {
    use crate::types::Row;

    match finding {
        "frame.jank_burst" | "frame.compositor_block" => {
            let mut rows: Vec<_> = data
                .frames
                .entries()
                .iter()
                .rev()
                .take(60)
                .filter(|frame| {
                    finding != "frame.compositor_block" || frame.compositor_block.is_some()
                })
                .collect();
            if finding == "frame.jank_burst" {
                rows.sort_by(|a, b| frames::duration(b).total_cmp(&frames::duration(a)));
            }
            rows.into_iter()
                .take(5)
                .map(|frame| Row {
                    id: frame.id.to_string(),
                    name: format!("Frame #{}", frame.id),
                    detail: format!(
                        "{:.2} ms / {:.2} ms budget",
                        frames::duration(frame),
                        frame.budget_ms
                    ),
                    ..Row::default()
                })
                .collect()
        }
        "view.repaint_hotspot" => {
            let mut rows: Vec<_> = views::recent_repaints(data).into_iter().collect();
            rows.sort_by_key(|(_, heat)| std::cmp::Reverse(heat.count));
            rows.into_iter()
                .take(5)
                .map(|(node, heat)| Row {
                    id: node.to_owned(),
                    name: format!("View {node}"),
                    detail: format!("{} repaints in 60 frames", heat.count),
                    ..Row::default()
                })
                .collect()
        }
        "view.layout_overflow" => {
            let mut rows: Vec<_> = data.layouts["issues"]
                .as_array()
                .into_iter()
                .flatten()
                .collect();
            rows.sort_by(|a, b| number(b, "excess").total_cmp(&number(a, "excess")));
            rows.into_iter()
                .take(5)
                .map(|issue| Row {
                    id: id(&issue["parent"]),
                    name: format!("View {}", id(&issue["parent"])),
                    detail: format!(
                        "{} · {:.1} px overflow",
                        string(issue, "axis"),
                        number(issue, "excess")
                    ),
                    ..Row::default()
                })
                .collect()
        }
        "net.slow_ttfb" | "net.error_rate" => data
            .network
            .values()
            .rev()
            .filter(|entry| {
                if finding == "net.error_rate" {
                    network::failed(entry)
                } else {
                    number(&entry["timing"], "waiting_ttfb_ms") > 800.
                        || network::elapsed(entry) > 3000.
                }
            })
            .take(5)
            .map(|entry| Row {
                id: id(&entry["token"]),
                name: crate::types::clip(string(entry, "url"), 100),
                detail: format!(
                    "{} · {:.0} ms",
                    id(&entry["status"]),
                    network::elapsed(entry)
                ),
                ..Row::default()
            })
            .collect(),
        _ => data
            .runtime
            .iter()
            .rev()
            .take(5)
            .map(|sample| Row {
                id: id(&sample["seq"]),
                name: "Heap sample".to_owned(),
                detail: format!(
                    "{:.2} MiB",
                    number(&sample["value"]["detail"], "memory_used_size") / 1048576.
                ),
                ..Row::default()
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devtools_protocol::FrameRecord;
    use serde_json::json;

    #[test]
    fn reports_jank_threshold_and_rate_boundaries() {
        let mut data = Recording::default();
        data.frames.update(
            false,
            (1..=3)
                .map(|id| FrameRecord {
                    id,
                    e2e_ms: 23.,
                    budget_ms: 16.,
                    over_budget: true,
                    ..FrameRecord::default()
                })
                .collect(),
        );
        for token in 1..=20 {
            data.network
                .insert(token, json!({"status": if token <= 3 {500} else {200}}));
        }
        let findings = evaluate(&data);
        assert_eq!(findings[0].id, "frame.jank_burst");
        assert!(
            findings
                .iter()
                .any(|finding| finding.id == "net.error_rate")
        );
        data.network.insert(21, json!({"status": 200}));
        assert!(
            evaluate(&data)
                .iter()
                .all(|finding| finding.id != "net.error_rate")
        );
    }
}
