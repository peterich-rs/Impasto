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
    WHERE q.name = 'queueBuffer' AND k.name = 'QueuePresentKHR'
      AND q.track_id = f.track_id AND q.ts BETWEEN f.ts AND f.ts + f.dur) AS queued
FROM slice f WHERE f.name = 'Impasto frame' AND f.dur > 0 ORDER BY f.ts
"""

MARKS = {
    "Impasto reactive after frame callbacks",
    "Impasto layout",
    "Impasto Canvas draw",
    "Impasto Canvas prepare",
    "Impasto ListOp",
    "Impasto record",
    "Impasto submit",
    "Impasto Vulkan acquire",
    "Impasto Vulkan draw",
    "Impasto Vulkan present",
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
        "peak_ms": round(ordered[-1] / 1e6, 2),
    }
    if budget_ns:
        stats["over_budget_percent"] = round(100 * sum(x > budget_ns for x in ordered) / len(ordered), 2)
    return stats


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


def load(path):
    with TraceProcessor(trace=str(path)) as processor:
        frames = list(processor.query(FRAME_SQL))
        clocks = list(processor.query("""
          SELECT ts - clock_value AS offset FROM clock_snapshot
          WHERE clock_name = 'MONOTONIC' ORDER BY ts LIMIT 1
        """))
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
    starts = [frame.ts for frame in frames]
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
            index = bisect.bisect_right(starts, marker.ts) - 1
            if index >= 0:
                frame = frames[index]
                if frame.present and frame.track_id == marker.track_id and marker.ts < frame.ts + frame.dur:
                    submitted_phases[marker.name].append(marker.dur)
    latency = []
    consumed = 0
    latest_input = None
    presented_frames = {}
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
        if index < len(desired) and desired[index] <= frame.queued + 2_000_000:
            presented_frames[frame.id] = presented[index][1]
            if latest_input is not None:
                elapsed = presented[index][1] - latest_input
                if 0 <= elapsed <= 500_000_000:
                    latency.append(elapsed)
        latest_input = None

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
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--hz", type=int, choices=(60, 120), required=True)
    args = parser.parse_args()
    budget = round(1e9 / args.hz)
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
        work, canvas = Counter(), Counter()
        for run in runs:
            work.update(run["work"])
            canvas.update(run["canvas_by_height"])
            for name, durations in run["phases"].items():
                phases[name].extend(durations)
            for name, durations in run["submitted_phases"].items():
                submitted_phases[name].extend(durations)
        report[case] = {
            "runs": len(runs),
            "submitted": distribution([x for run in runs for x in run["frames"]], budget),
            "new_layer_trees": sum(run["installs"] for run in runs),
            "skipped": sum(run["skipped"] for run in runs),
            "actual_presentation_intervals": distribution([x for run in runs for x in run["intervals"]]),
            "input_to_presentation": distribution([x for run in runs for x in run["latency"]]),
            "first_content_visible": distribution([x for run in runs for x in run["first_content"]]),
            "queued_buffers": sum(run["queued"] for run in runs),
            "presented_buffers": sum(run["presented"] for run in runs),
            "matched_presentations": sum(run["matched_presentations"] for run in runs),
            "refresh_intervals_ns": sorted({x for run in runs for x in run["refreshes"]}),
            "work": work,
            "canvas_commits_by_height": canvas,
            "phases": {name: distribution(values) for name, values in sorted(phases.items())},
            "submitted_phases": {name: distribution(values) for name, values in sorted(submitted_phases.items())},
        }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
