#!/usr/bin/env python3
"""Summarize Gallery compositor and UI slices from paired Android Perfetto traces."""

import argparse
import json
from pathlib import Path
import statistics

from perfetto.trace_processor import TraceProcessor


SQL = """
WITH compositor AS (
  SELECT ts, dur, ts - lag(ts) OVER (ORDER BY ts) AS gap
  FROM slice WHERE name = 'compositor_frame' AND dur > 0
), interval AS (
  SELECT min(ts) AS lo, max(ts + dur) AS hi FROM compositor
)
SELECT count(*) AS compositor_frames,
       round(percentile(dur / 1e6, 50), 3) AS raster_p50_ms,
       round(percentile(dur / 1e6, 95), 3) AS raster_p95_ms,
       round(percentile(gap / 1e6, 50), 3) AS cadence_p50_ms,
       round(percentile(gap / 1e6, 95), 3) AS cadence_p95_ms,
       count(CASE WHEN dur > 8333333 THEN 1 END) AS over_8_33_ms,
       count(CASE WHEN gap > 12500000 THEN 1 END) AS gaps_over_12_5_ms,
       (SELECT count(*) FROM slice, interval
        WHERE name = 'Impasto frame' AND ts BETWEEN lo AND hi) AS ui_frames
FROM compositor
"""


def read_trace(path):
    with TraceProcessor(trace=str(path)) as trace:
        metrics = vars(next(iter(trace.query(SQL))))
    if metrics["compositor_frames"] < 2:
        raise ValueError(f"{path}: need at least two compositor_frame slices")
    return metrics


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    options = parser.parse_args()
    if options.rounds < 1:
        parser.error("--rounds must be positive")
    traces = {
        path.stem: read_trace(path)
        for path in sorted(options.directory.glob("*.perfetto-trace"))
    }
    medians = {}
    for scene in ("scroll", "transform"):
        medians[scene] = {}
        for side in ("base", "head"):
            names = [f"{side}-{scene}-{run}" for run in range(1, options.rounds + 1)]
            missing = [name for name in names if name not in traces]
            if missing:
                parser.error(f"missing paired traces: {', '.join(missing)}")
            medians[scene][side] = {
                metric: statistics.median(traces[name][metric] for name in names)
                for metric in traces[names[0]]
            }
    print(json.dumps({"traces": traces, "medians": medians}, indent=2))


if __name__ == "__main__":
    main()
