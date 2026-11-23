#!/usr/bin/env python3
"""Compare four raster benchmarks from two prepared Impasto checkouts."""

import argparse
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys


SCENES = {
    "scroll_1000_cards", "dense_text_wall", "backdrop_blur_overlays", "canvas_500_paths",
}
PREFIX = "IMPASTO_BENCH_JSON "


def capture(command, checkout, env=None):
    result = subprocess.run(command, cwd=checkout, env=env, text=True, stdout=subprocess.PIPE)
    if result.returncode:
        print(result.stdout, file=sys.stderr, end="")
        result.check_returncode()
    return result.stdout


def prepare(checkout):
    print(f"Precompiling {checkout}", file=sys.stderr)
    output = capture([
        "cargo", "test", "--locked", "-p", "raster", "--lib", "--release", "--no-run",
        "--message-format=json",
    ], checkout)
    for line in output.splitlines():
        artifact = json.loads(line)
        if (artifact.get("reason") == "compiler-artifact"
                and artifact.get("profile", {}).get("test")
                and artifact.get("target", {}).get("name") == "raster"):
            binary = Path(artifact["executable"])
            sha = capture(["git", "rev-parse", "HEAD"], checkout).strip()
            if capture(["git", "status", "--porcelain", "--untracked-files=no"], checkout).strip():
                sha += "-dirty"
            return binary, sha
    raise ValueError(f"raster benchmark binary missing in {checkout}")


def run(checkout, prepared, backend, gpu):
    binary, sha = prepared
    env = {
        **os.environ,
        "IMPASTO_BENCH_GIT_SHA": sha,
        "IMPASTO_BENCH_BACKEND": backend,
        "IMPASTO_BENCH_GPU": "1" if gpu else "0",
    }
    output = capture([
        str(binary), "tests::p0::bench_", "--ignored", "--nocapture", "--test-threads=1",
    ], checkout, env)
    reports = {}
    for line in output.splitlines():
        _, marker, payload = line.partition(PREFIX)
        if marker:
            report = json.loads(payload)
            scene = report["scene"]
            if scene in reports or report["schema"] != 1:
                raise ValueError(f"duplicate scene or unsupported schema in {checkout}: {scene}")
            if report["environment"]["debug_assertions"]:
                raise ValueError(f"{checkout} did not run with release assertions")
            reports[scene] = report
    if reports.keys() != SCENES:
        raise ValueError(f"{checkout} must have the Stage 0 harness, got scenes {sorted(reports)}")
    return reports


def fingerprint(report):
    return {
        **{key: value for key, value in report["environment"].items() if key != "git_sha"},
        "viewport": report["viewport"],
        "dpr": report["dpr"],
        "sample_count": len(report["samples"]),
    }


def compare(pairs):
    summaries = {}
    for scene in sorted(SCENES):
        runs = [(pair["base"][scene], pair["head"][scene]) for pair in pairs]
        reference = fingerprint(runs[0][0])
        if any(fingerprint(report) != reference for pair in runs for report in pair):
            raise ValueError(f"environment or scene configuration differs for {scene}")
        common = set.intersection(*(set(report["steady_state_us"]) for pair in runs for report in pair))
        summary = {}
        for metric in sorted(common):
            values = {}
            for percentile in ("p50", "p95"):
                timings = [
                    (base["steady_state_us"][metric][percentile],
                     head["steady_state_us"][metric][percentile])
                    for base, head in runs
                ]
                deltas = [(head / base - 1) * 100 for base, head in timings if base > 0]
                values[percentile] = {
                    "base_median_us": statistics.median(base for base, _ in timings),
                    "head_median_us": statistics.median(head for _, head in timings),
                    "paired_delta_pct": deltas,
                    "median_delta_pct": statistics.median(deltas) if deltas else None,
                }
            values["sample_counts"] = [
                {"base": base["steady_state_us"][metric]["samples"],
                 "head": head["steady_state_us"][metric]["samples"]}
                for base, head in runs
            ]
            summary[metric] = values
        summaries[scene] = summary
    return summaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path, help="checkout containing the baseline")
    parser.add_argument("head", type=Path, help="checkout containing the candidate")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--backend", choices=["auto", "metal", "vulkan", "gles", "dx12"], default="auto")
    parser.add_argument("--cpu-only", action="store_true")
    parser.add_argument("--out", type=Path, required=True, help="JSON report path")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    checkouts = {"base": args.base.resolve(), "head": args.head.resolve()}
    builds = {path: prepare(path) for path in dict.fromkeys(checkouts.values())}
    if checkouts["base"] != checkouts["head"] and builds[checkouts["base"]][0] == builds[checkouts["head"]][0]:
        raise ValueError("checkouts share a benchmark binary path, use separate Cargo target directories")
    pairs = []
    for index in range(args.rounds):
        order = ["base", "head"] if index % 2 == 0 else ["head", "base"]
        print(f"Pair {index + 1}/{args.rounds}: {' → '.join(order)}", file=sys.stderr)
        pair = {"order": order}
        for side in order:
            path = checkouts[side]
            pair[side] = run(path, builds[path], args.backend, not args.cpu_only)
        pairs.append(pair)
    summary = compare(pairs)
    report = {
        "schema": 1,
        "host": {"platform": platform.platform(), "processor": platform.processor()},
        "pairs": pairs,
        "summary": summary,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    for scene, metrics in summary.items():
        pieces = []
        for metric in ("encode_us", "gpu_frame_span_us"):
            value = metrics.get(metric, {}).get("p50", {}).get("median_delta_pct")
            if value is not None:
                pieces.append(f"{metric} {value:+.1f}%")
        print(f"{scene}: {', '.join(pieces)}")
    print(args.out)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
