use devtools_protocol::FrameRecord;
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
    let budget = data.frame_budget();
    let stats = frames::stats(recent(), budget);
    let severe_stage = recent().any(|frame| {
        let budget = frames::effective_budget(frame, budget);
        budget > 0. && frames::stage_duration(frame) > 2. * budget
    });
    let severity = if severe_stage || stats.dropped_frames >= 3 || stats.hitch_ms_per_sec >= 10. {
        Some("critical")
    } else if stats.hitch_ms_per_sec >= 5. {
        Some("warning")
    } else {
        None
    };
    if let Some(severity) = severity
        && let Some(frame) = recent().max_by(|a, b| compare_jank(a, b, budget))
    {
        let (stage, count) = [
            ("UI", stats.ui_jank_count),
            ("Raster", stats.raster_jank_count),
            ("GPU", stats.gpu_jank_count),
        ]
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or(("Present", 0));
        let stage = if count > 0 { stage } else { "Present" };
        add(
            "frame.jank_burst",
            severity,
            "Frame hitch burst",
            format!(
                "{:.1} ms/s hitch · {} dropped ({stage})",
                stats.hitch_ms_per_sec, stats.dropped_frames
            ),
            "Inspect the frame with the most dropped refresh intervals.",
            "frame",
            frame.id.to_string(),
            "breakdown",
        );
    }
    let (observed, seconds) = repaint_window(data);
    if observed.len() >= 10 && seconds >= 2. {
        let wakes: Vec<_> = observed
            .into_iter()
            .filter(|frame| idle_repaint(frame))
            .collect();
        let wake_hz = wakes.len() as f64 / seconds;
        if wake_hz >= 2.
            && let Some(frame) = wakes
                .iter()
                .max_by(|a, b| frames::stage_duration(a).total_cmp(&frames::stage_duration(b)))
        {
            let count = wakes.len() as f64;
            let (total_views, measured) = wakes
                .iter()
                .filter_map(|frame| frame.repainted_view_count)
                .fold((0., 0.), |(total, measured), views| {
                    (total + f64::from(views), measured + 1.)
                });
            let paint = if measured > 0. {
                format!("{:.1} views/frame", total_views / measured)
            } else {
                "— views/frame".to_owned()
            };
            let timer_ms = wakes.iter().map(|frame| frame.timer_ms).sum::<f64>() / count;
            add(
                "frame.idle_repaint_churn",
                if wake_hz >= 20. { "warning" } else { "info" },
                "Background repaints",
                format!("{wake_hz:.1}/s · {paint} · {timer_ms:.2} ms timer/frame"),
                "Inspect frames that woke without input to find periodic repaint work.",
                "frame",
                frame.id.to_string(),
                "breakdown",
            );
        }
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

fn compare_jank(a: &FrameRecord, b: &FrameRecord, budget: f64) -> std::cmp::Ordering {
    frames::dropped_vsyncs(a, budget)
        .cmp(&frames::dropped_vsyncs(b, budget))
        .then_with(|| frames::stage_duration(a).total_cmp(&frames::stage_duration(b)))
}

fn repaint_window(data: &Recording) -> (Vec<&FrameRecord>, f64) {
    // At 60 Hz the last 60 frames span only one second.
    const WINDOW_MS: f64 = 5_000.;
    let entries = data.frames.entries();
    let end = entries.iter().map(|frame| frame.at_ms).fold(0., f64::max);
    let mut start = end;
    let frames = entries
        .iter()
        .filter(|frame| frame.at_ms >= end - WINDOW_MS)
        .inspect(|frame| start = start.min(frame.at_ms))
        .collect();
    (frames, (end - start) / 1_000.)
}

fn idle_repaint(frame: &FrameRecord) -> bool {
    frame.trigger == "wake"
        && frame.input_events == 0
        && !frame.compositor_only
        && !frames::failed_or_skipped(frame)
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
                rows.sort_by(|a, b| compare_jank(b, a, data.frame_budget()));
            }
            rows.into_iter()
                .take(5)
                .map(|frame| Row {
                    id: frame.id.to_string(),
                    name: format!("Frame #{}", frame.id),
                    detail: format!(
                        "{:.2} ms / {:.2} ms budget",
                        frames::stage_duration(frame),
                        frames::effective_budget(frame, data.frame_budget())
                    ),
                    ..Row::default()
                })
                .collect()
        }
        "frame.idle_repaint_churn" => {
            let (observed, _) = repaint_window(data);
            let mut rows: Vec<_> = observed
                .into_iter()
                .filter(|frame| idle_repaint(frame))
                .collect();
            rows.sort_by(|a, b| frames::stage_duration(b).total_cmp(&frames::stage_duration(a)));
            rows.into_iter()
                .take(5)
                .map(|frame| Row {
                    id: frame.id.to_string(),
                    name: format!("Frame #{} · wake", frame.id),
                    detail: format!(
                        "{} · {:.2} ms timer · {} repainted views",
                        frame.layout_cause,
                        frame.timer_ms,
                        frame
                            .repainted_view_count
                            .map_or_else(|| "—".to_owned(), |count| count.to_string())
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
                    at_ms: id as f64 * 16.,
                    presented: true,
                    raster_ms: 23.,
                    budget_ms: 16.,
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
    #[test]
    fn hitch_audit_uses_warning_and_critical_boundaries() {
        for (span, expected) in [
            (4000., None),
            (3200., Some("warning")),
            (1600., Some("critical")),
        ] {
            let mut data = Recording::default();
            data.frames.update(
                false,
                vec![
                    FrameRecord {
                        id: 1,
                        presented: true,
                        budget_ms: 16.,
                        ui_ms: 12.,
                        raster_ms: 12.,
                        e2e_ms: 35.,
                        ..FrameRecord::default()
                    },
                    FrameRecord {
                        id: 2,
                        at_ms: span,
                        presented: true,
                        budget_ms: 16.,
                        gpu: devtools_protocol::GpuTiming {
                            frame_span_us: Some(25_000.),
                            ..Default::default()
                        },
                        ..FrameRecord::default()
                    },
                ],
            );
            let findings = evaluate(&data);
            let finding = findings
                .iter()
                .find(|finding| finding.id == "frame.jank_burst");
            assert_eq!(finding.map(|value| value.severity.as_str()), expected);
            if let Some(finding) = finding {
                assert!(finding.metric.ends_with("1 dropped (GPU)"));
                assert_eq!(finding.selection, "2");
            }
        }
    }

    #[test]
    fn idle_repaint_audit_uses_input_and_two_seconds_of_observation() {
        for (interval, count, expected) in [
            (200., 10, None),
            (200., 11, Some("info")),
            (50., 41, Some("warning")),
        ] {
            let mut data = Recording::default();
            data.frames.update(
                false,
                (0..count)
                    .map(|id| FrameRecord {
                        id,
                        at_ms: id as f64 * interval,
                        presented: true,
                        budget_ms: 16.,
                        trigger: "wake".to_owned(),
                        layout_cause: "timer".to_owned(),
                        timer_ms: 0.5,
                        repainted_view_count: (id % 2 == 0).then_some(3),
                        raster_ms: 4.,
                        ..FrameRecord::default()
                    })
                    .collect(),
            );
            let findings = evaluate(&data);
            let finding = findings
                .iter()
                .find(|finding| finding.id == "frame.idle_repaint_churn");
            assert_eq!(finding.map(|value| value.severity.as_str()), expected);
            if let Some(finding) = finding {
                assert!(finding.metric.contains("3.0 views/frame"));
                assert!(
                    evidence(&data, &finding.id)
                        .iter()
                        .any(|row| row.detail.contains("0.50 ms timer · 3 repainted views"))
                );
                assert!(
                    evidence(&data, &finding.id)
                        .iter()
                        .any(|row| row.detail.contains("0.50 ms timer · — repainted views"))
                );
            }
            let interactive = data
                .frames
                .entries()
                .iter()
                .cloned()
                .map(|mut frame| {
                    frame.input_events = 1;
                    frame
                })
                .collect();
            data.frames.update(true, interactive);
            assert!(
                evaluate(&data)
                    .iter()
                    .all(|finding| finding.id != "frame.idle_repaint_churn")
            );
        }
    }
}
