#!/usr/bin/env python3
"""Compare acquire latency and compositor cadence in paired Perfetto traces."""

import argparse
import json
import math
from pathlib import Path
import statistics

from perfetto.trace_processor import TraceProcessor


SQL = """
WITH start AS (
  SELECT min(ts) AS ts FROM slice WHERE name = 'compositor_frame' AND dur > 0
), compositor AS (
  SELECT frame.ts, frame.dur, frame.ts - lag(frame.ts) OVER (ORDER BY frame.ts) AS gap
  FROM slice frame, start
  WHERE frame.name = 'compositor_frame' AND frame.dur > 0
    AND frame.ts >= start.ts + {skip_ns}
    AND ({duration_ns} = 0 OR frame.ts < start.ts + {skip_ns} + {duration_ns})
), interval AS (
  SELECT min(ts) AS lo, max(ts + dur) AS hi FROM compositor
), acquire AS (
  SELECT dur FROM slice, interval
  WHERE name = 'surface_acquire' AND dur > 0 AND ts BETWEEN lo AND hi
)
SELECT count(*) AS compositor_frames,
       round(percentile(dur / 1e6, 50), 3) AS raster_p50_ms,
       round(percentile(dur / 1e6, 95), 3) AS raster_p95_ms,
       round(percentile(gap / 1e6, 50), 3) AS cadence_p50_ms,
       round(percentile(gap / 1e6, 95), 3) AS cadence_p95_ms,
       round(percentile(gap / 1e6, 99), 3) AS cadence_p99_ms,
       (SELECT count(*) FROM acquire) AS acquire_samples,
       (SELECT round(percentile(dur / 1e6, 95), 3) FROM acquire) AS acquire_p95_ms,
       count(CASE WHEN dur > 8333333 THEN 1 END) AS over_8_33_ms,
       count(CASE WHEN gap > 12500000 THEN 1 END) AS gaps_over_12_5_ms,
       (SELECT count(*) FROM slice, interval
        WHERE name = 'Impasto frame' AND ts BETWEEN lo AND hi) AS ui_frames
FROM compositor
"""


def read_trace(path, skip_ns, duration_ns):
    with TraceProcessor(trace=str(path)) as trace:
        metrics = vars(next(iter(trace.query(SQL.format(skip_ns=skip_ns, duration_ns=duration_ns)))))
    if metrics["compositor_frames"] < 2:
        raise ValueError(f"{path}: need at least two compositor_frame slices")
    return metrics


def nonnegative_seconds(value):
    seconds = float(value)
    if not math.isfinite(seconds) or seconds < 0:
        raise argparse.ArgumentTypeError("expected a finite nonnegative number")
    return seconds


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--scene", action="append", help="trace names to compare (default: scroll and transform)")
    parser.add_argument("--skip-seconds", type=nonnegative_seconds, default=0)
    parser.add_argument("--duration-seconds", type=nonnegative_seconds, default=0,
                        help="restrict the sample to a known high-refresh interval")
    parser.add_argument("--max-acquire-p95-ms", type=nonnegative_seconds)
    parser.add_argument("--max-cadence-p95-ms", type=nonnegative_seconds)
    options = parser.parse_args()
    if options.rounds < 1:
        parser.error("--rounds must be positive")
    traces = {}
    medians = {}
    failures = []
    skip_ns = round(options.skip_seconds * 1e9)
    duration_ns = round(options.duration_seconds * 1e9)
    for scene in options.scene or ("scroll", "transform"):
        medians[scene] = {}
        for side in ("base", "head"):
            names = [f"{side}-{scene}-{run}" for run in range(1, options.rounds + 1)]
            for name in names:
                path = options.directory / f"{name}.perfetto-trace"
                if not path.is_file():
                    parser.error(f"missing paired trace: {path}")
                metrics = read_trace(path, skip_ns, duration_ns)
                traces[name] = metrics
                if side == "head":
                    for metric, limit in (("acquire_p95_ms", options.max_acquire_p95_ms),
                                          ("cadence_p95_ms", options.max_cadence_p95_ms)):
                        value = metrics[metric]
                        if limit is not None and (value is None or value > limit):
                            failures.append(f"{name}: {metric} = {value}, maximum {limit}")
            medians[scene][side] = {
                metric: statistics.median(values) if values else None
                for metric in traces[names[0]]
                for values in [[traces[name][metric] for name in names if traces[name][metric] is not None]]
            }
    print(json.dumps({"traces": traces, "medians": medians}, indent=2))
    if failures:
        raise SystemExit("\n".join(failures))


if __name__ == "__main__":
    main()
