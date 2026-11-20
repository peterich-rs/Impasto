#!/usr/bin/env python3
"""Read the Perfetto captures and SurfaceFlinger presentation times from android-frame-replay.sh."""

import argparse
import bisect
from collections import Counter, defaultdict
import json
import re
from pathlib import Path

from perfetto.trace_processor import TraceProcessor

FRAME_SQL = """
SELECT f.ts, f.dur, f.track_id, f.id,
  (SELECT MIN(p.ts) FROM slice p WHERE p.parent_id = f.id AND p.name = 'Impasto input') AS input_ts,
  EXISTS(SELECT 1 FROM slice p WHERE p.parent_id = f.id AND p.name = 'Impasto submit') AS install,
  EXISTS(SELECT 1 FROM slice p WHERE p.parent_id = f.id AND p.name = 'Impasto present') AS present,
  (SELECT MIN(q.ts) FROM slice q JOIN slice k ON q.parent_id = k.id
    WHERE q.name = 'queueBuffer' AND k.name IN ('QueuePresentKHR', 'eglSwapBuffers', 'Impasto GLES draw and swap')
      AND q.track_id = f.track_id AND q.ts BETWEEN f.ts AND f.ts + f.dur) AS queued
FROM slice f WHERE f.name = 'Impasto frame' AND f.dur > 0 ORDER BY f.ts
"""
CLOCK_SQL = """
SELECT ts - clock_value AS offset FROM clock_snapshot
WHERE clock_name = 'MONOTONIC' ORDER BY ts LIMIT 1
"""

MARKS = {
    "Impasto reactive after frame callbacks",
    "Impasto layout",
    "Impasto Canvas draw",
    "Impasto Canvas prepare",
    "Impasto ListOp",
    "Impasto mutation flush",
    "Impasto record",
    "Impasto submit",
    "Impasto Vulkan acquire",
    "Impasto Vulkan draw",
    "Impasto Vulkan present",
    "Impasto GLES draw and swap",
}

FRAME_EVENTS = {
    "Impasto frame id=": "clock",
    "Impasto work ": "work",
    "Impasto root ": "root",
    "Impasto layout work ": "layout",
    "Impasto text probes ": "text_probes",
    "Impasto script work ": "script",
    "Impasto record work ": "record",
    "Impasto scroll work ": "scroll",
    "Impasto pictures ": "pictures",
}


def distribution(samples, budget_ns=None):
    if not samples:
        return {"count": 0}
    ordered = sorted(samples)

    def percentile(fraction):
        position = (len(ordered) - 1) * fraction
        lower = int(position)
        upper = min(lower + 1, len(ordered) - 1)
        return round((ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)) / 1e6, 2)

    stats = {
        "count": len(ordered),
        "p50_ms": percentile(0.5),
        "p95_ms": percentile(0.95),
        "p99_ms": percentile(0.99),
        "peak_ms": round(ordered[-1] / 1e6, 2),
    }
    if budget_ns:
        stats["over_budget_percent"] = round(100 * sum(x > budget_ns for x in ordered) / len(ordered), 2)
    return stats


def missed_periods(intervals, period_ns):
    periods = [max(1, round(interval / period_ns)) for interval in intervals]
    if not periods:
        return {"matched_intervals": 0}
    expected = sum(periods)
    missed = expected - len(periods)
    return {
        "matched_intervals": len(periods),
        "expected_periods": expected,
        "missed_periods": missed,
        "missed_percent": round(100 * missed / expected, 2),
    }


def read_presentations(path, offset, start, end):
    # SurfaceFlinger returns desired, actual, ready in monotonic nanoseconds.
    # A pending fence uses INT64_MAX. Samples overlap by design.
    if not path.exists() or offset is None:
        return [], []
    records = set()
    refreshes = set()
    for line in path.read_text().splitlines():
        if re.fullmatch(r"[0-9]+", line):
            refreshes.add(int(line))
        fields = line.split()
        if len(fields) == 3 and all(field.isdecimal() for field in fields):
            desired, actual, ready = map(int, fields)
            if all(0 < stamp < 2**63 - 1 for stamp in (desired, actual, ready)):
                if start <= desired + offset <= end:
                    records.add((desired + offset, actual + offset))
    return sorted(records), sorted(refreshes)


def flutter_report(directory, budget):
    cases = defaultdict(list)
    for path in sorted(directory.glob("*.perfetto-trace")):
        with TraceProcessor(trace=str(path)) as processor:
            frames = list(processor.query("""
              SELECT name, ts, dur FROM slice
              WHERE name IN ('Animator::BeginFrame', 'GPURasterizer::Draw') AND dur > 0
              ORDER BY ts
            """))
            clocks = list(processor.query(CLOCK_SQL))
        if not frames:
            raise ValueError(f"{path} has no Flutter frames (launch with trace-systrace)")
        offset = clocks[0].offset if clocks else None
        end = max(frame.ts + frame.dur for frame in frames)
        presented, refreshes = read_presentations(path.with_suffix(".presentation"), offset, frames[0].ts, end)
        actual = sorted({stamp for _, stamp in presented})
        case = path.stem.rsplit("-", 1)[0]
        cases[case].append({
            "ui": [frame.dur for frame in frames if frame.name == "Animator::BeginFrame"],
            "raster": [frame.dur for frame in frames if frame.name == "GPURasterizer::Draw"],
            "intervals": [after - before for before, after in zip(actual, actual[1:])],
            "refreshes": refreshes,
        })
    return {
        case: {
            "runs": len(runs),
            "ui_frame": distribution([x for run in runs for x in run["ui"]], budget),
            "raster_frame": distribution([x for run in runs for x in run["raster"]], budget),
            "actual_presentation_intervals": distribution([x for run in runs for x in run["intervals"]]),
            "refresh_intervals_ns": sorted({x for run in runs for x in run["refreshes"]}),
        }
        for case, runs in sorted(cases.items())
    }


def load(path):
    with TraceProcessor(trace=str(path)) as processor:
        frames = list(processor.query(FRAME_SQL))
        clocks = list(processor.query(CLOCK_SQL))
        markers = list(processor.query("""
          SELECT ts, dur, name, track_id FROM slice
          WHERE name GLOB 'Impasto *' OR name GLOB 'deliverInputEvent src=0x1002 eventTimeNano=*'
            OR name = 'launching: dev.impasto.host.profile'
          ORDER BY ts
        """))
    if not frames:
        raise ValueError(f"{path} has no Impasto frames")
    begin = frames[0].ts
    end = frames[-1].ts + frames[-1].dur
    offset = clocks[0].offset if clocks else None
    presented, refreshes = read_presentations(path.with_suffix(".presentation"), offset, begin, end)
    desired = [item[0] for item in presented]
    actual = sorted({item[1] for item in presented})
    deliveries = []
    phases = defaultdict(list)
    submitted_phases = defaultdict(list)
    counts = Counter()
    chart = Counter()
    chart_markers = {}
    frame_events = defaultdict(dict)
    starts = [frame.ts for frame in frames]

    def containing_frame(marker):
        index = bisect.bisect_right(starts, marker.ts) - 1
        if index >= 0:
            frame = frames[index]
            if frame.track_id == marker.track_id and marker.ts < frame.ts + frame.dur:
                return frame
        return None

    launch = None
    for marker in markers:
        if marker.name == 'launching: dev.impasto.host.profile':
            launch = marker.ts
        elif marker.name.startswith("deliverInputEvent src=") and offset is not None:
            match = re.search(r"eventTimeNano=(\d+)", marker.name)
            if match:
                deliveries.append((marker.ts, int(match[1]) + offset))
        elif marker.name.startswith("Impasto work "):
            counts.update({key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", marker.name)})
        elif marker.name.startswith("Impasto Canvas commit "):
            match = re.search(r"h=(\d+(?:\.\d+)?)", marker.name)
            if match:
                chart[match[1]] += 1
                chart_markers.setdefault(match[1], marker.ts)
        elif marker.name in MARKS and marker.dur > 0:
            phases[marker.name].append(marker.dur)
            frame = containing_frame(marker)
            if frame is not None and frame.present:
                submitted_phases[marker.name].append(marker.dur)
        for prefix, category in FRAME_EVENTS.items():
            if marker.name.startswith(prefix):
                frame = containing_frame(marker)
                if frame is not None:
                    frame_events[frame.id][category] = dict(
                        re.findall(r"(\w+)=(\S+)", marker.name)
                    )
                break
    latency = []
    consumed = 0
    latest_input = None
    presented_frames = {}
    matched_buffers = set()
    for frame in frames:
        if frame.input_ts is not None:
            while consumed < len(deliveries) and deliveries[consumed][0] <= frame.input_ts:
                latest_input = deliveries[consumed][1]
                consumed += 1
        if frame.queued is None:
            continue
        # Desired timestamp is assigned around queueBuffer. Restrict the
        # match to the same call window and discard ambiguous/missing pairs.
        index = bisect.bisect_left(desired, frame.queued - 500_000)
        if index + 1 < len(desired) and desired[index + 1] <= frame.queued + 2_000_000:
            index = len(desired)
        if index < len(desired) and index not in matched_buffers and desired[index] <= frame.queued + 2_000_000:
            matched_buffers.add(index)
            presented_frames[frame.id] = presented[index][1]
            if latest_input is not None:
                elapsed = presented[index][1] - latest_input
                if 0 <= elapsed <= 500_000_000:
                    latency.append(elapsed)
        latest_input = None

    scroll_frames = defaultdict(list)
    scroll_intervals = []
    scroll_start_delays = []
    previous_scroll_present = None
    diagnostics = defaultdict(list)
    for frame in frames:
        if not frame.present:
            continue
        events = frame_events.get(frame.id, {})
        for category, fields in events.items():
            diagnostics[category].append(fields)
        if "scroll" not in events:
            previous_scroll_present = None
            continue
        work = events.get("work", {})
        if int(work.get("roots", 0)):
            group = "root_layout"
        elif int(work.get("draw", 0)):
            group = "canvas_without_root"
        else:
            group = "without_root_or_canvas"
        scroll_frames[group].append(frame.dur)
        vsync_us = events.get("clock", {}).get("vsync_us", "")
        if offset is not None and vsync_us.isdecimal():
            start_delay = frame.ts - (int(vsync_us) * 1000 + offset)
            if start_delay >= 0:
                scroll_start_delays.append(start_delay)
        # An unmatched submission breaks the pair of display timestamps.
        actual_ts = presented_frames.get(frame.id)
        if previous_scroll_present is not None and actual_ts is not None:
            interval = actual_ts - previous_scroll_present
            if interval > 0:
                scroll_intervals.append(interval)
        previous_scroll_present = actual_ts

    # The first screen milestone is the first stock sparkline on the surface.
    # Entry is the first chart frame after the replay taps 00700.
    first_content = []
    case = path.stem.rsplit("-", 1)[0]
    if case == "startup":
        start, marker = launch, chart_markers.get("36")
    elif case == "detail-entry" and deliveries:
        start, marker = deliveries[0][1], chart_markers.get("204")
    else:
        start, marker = None, None
    if start is not None and marker is not None:
        frame_index = bisect.bisect_right(starts, marker) - 1
        if frame_index >= 0:
            visible = presented_frames.get(frames[frame_index].id)
            if visible is not None and visible >= start:
                first_content.append(visible - start)
    return {
        "frames": [frame.dur for frame in frames if frame.present],
        "installs": sum(frame.install for frame in frames),
        "skipped": sum(not frame.present for frame in frames),
        "queued": sum(frame.queued is not None for frame in frames),
        "presented": len(presented),
        "matched_presentations": len(presented_frames),
        "first_content": first_content,
        "intervals": [end - start for start, end in zip(actual, actual[1:])],
        "latency": latency,
        "refreshes": refreshes,
        "work": counts,
        "canvas_by_height": chart,
        "phases": phases,
        "submitted_phases": submitted_phases,
        "diagnostics": diagnostics,
        "scroll_frames": scroll_frames,
        "scroll_intervals": scroll_intervals,
        "scroll_start_delays": scroll_start_delays,
    }


def summarize_diagnostics(runs):
    events = defaultdict(list)
    for run in runs:
        for category, rows in run["diagnostics"].items():
            events[category].extend(rows)
    root_sources = Counter()
    root_rects = {}
    for row in events["root"]:
        key = row["cause"], row["node"]
        root_sources[key] += 1
        if all(field in row for field in ("x", "y", "w", "h")):
            root_rects.setdefault(key, [int(row[field]) for field in ("x", "y", "w", "h")])
    result = {"root_sources": []}
    for (cause, node), count in root_sources.most_common(10):
        root = {"cause": cause, "node": node, "frames": count}
        if (cause, node) in root_rects:
            root["sample_rect_dp"] = root_rects[cause, node]
        result["root_sources"].append(root)
    for category in ("layout", "text_probes", "script", "record", "pictures"):
        if not events[category]:
            continue
        values = defaultdict(list)
        for row in events[category]:
            for key, value in row.items():
                if value.isdecimal():
                    values[key].append(int(value))
        result[category] = {
            key: distribution([value * 1000 for value in samples])
            if key.endswith("_us")
            else {"frames": len(samples), "total": sum(samples), "peak": max(samples)}
            for key, samples in values.items()
        }
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--hz", type=int, choices=(60, 90, 120), required=True)
    parser.add_argument("--reference", choices=("impasto", "flutter"), default="impasto")
    args = parser.parse_args()
    budget = round(1e9 / args.hz)
    if args.reference == "flutter":
        report = flutter_report(args.directory, budget)
        if not report:
            parser.error("no replay traces found")
        print(json.dumps(report, indent=2))
        return
    cases = defaultdict(list)
    for path in sorted(args.directory.glob("*.perfetto-trace")):
        case = path.stem.rsplit("-", 1)[0]
        cases[case].append(load(path))
    if not cases:
        parser.error("no replay traces found")
    report = {}
    for case, runs in sorted(cases.items()):
        phases = defaultdict(list)
        submitted_phases = defaultdict(list)
        scroll_frames = defaultdict(list)
        work, canvas = Counter(), Counter()
        for run in runs:
            work.update(run["work"])
            canvas.update(run["canvas_by_height"])
            for name, durations in run["phases"].items():
                phases[name].extend(durations)
            for name, durations in run["submitted_phases"].items():
                submitted_phases[name].extend(durations)
            for name, durations in run["scroll_frames"].items():
                scroll_frames[name].extend(durations)
        queued = sum(run["queued"] for run in runs)
        matched = sum(run["matched_presentations"] for run in runs)
        report[case] = {
            "runs": len(runs),
            "submitted": distribution([x for run in runs for x in run["frames"]], budget),
            "new_layer_trees": sum(run["installs"] for run in runs),
            "skipped": sum(run["skipped"] for run in runs),
            "actual_presentation_intervals": distribution([x for run in runs for x in run["intervals"]]),
            "input_to_presentation": distribution([x for run in runs for x in run["latency"]]),
            "first_content_visible": distribution([x for run in runs for x in run["first_content"]]),
            "queued_buffers": queued,
            "presented_buffers": sum(run["presented"] for run in runs),
            "matched_presentations": matched,
            "matched_percent_of_queued": round(100 * matched / queued, 2) if queued else None,
            "refresh_intervals_ns": sorted({x for run in runs for x in run["refreshes"]}),
            "work": work,
            "canvas_commits_by_height": canvas,
            "phases": {name: distribution(values) for name, values in sorted(phases.items())},
            "submitted_phases": {name: distribution(values) for name, values in sorted(submitted_phases.items())},
            "diagnostics": summarize_diagnostics(runs),
        }
        if scroll_frames:
            scroll_intervals = [ns for run in runs for ns in run["scroll_intervals"]]
            report[case]["scrolling"] = {
                "frames": distribution([ns for group in scroll_frames.values() for ns in group], budget),
                "by_work": {name: distribution(group, budget) for name, group in sorted(scroll_frames.items())},
                "present_intervals": distribution(scroll_intervals),
                "vsync_to_render_start": distribution([ns for run in runs for ns in run["scroll_start_delays"]]),
            }
            measured = report[case]["refresh_intervals_ns"]
            if measured and all(abs(value - budget) / budget < 0.01 for value in measured):
                report[case]["scrolling"]["present_periods"] = missed_periods(scroll_intervals, budget)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
