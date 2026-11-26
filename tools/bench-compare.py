#!/usr/bin/env python3
"""Compare Impasto raster benchmarks in alternating runs."""

import argparse
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys


SCENES = {
    "scroll_1000_cards", "scroll_10000_cards", "dense_text_wall",
    "backdrop_blur_overlays", "canvas_500_paths",
}
WAVE_SCENES = {
    "hetero_cards_50", "opaque_modal_stack", "text_scale_and_cjk",
    "blur_and_mip_matrix", "canvas_mixed_paths",
}
SUITES = {
    "tests::p0::bench_": SCENES,
    "tests::p0::wave::bench_": WAVE_SCENES,
}
PREFIX = "IMPASTO_BENCH_JSON "
EXPERIMENTS = {
    "IMPASTO_CANVAS_CURVES", "IMPASTO_CANVAS_DIRECT", "IMPASTO_CANVAS_STENCIL", "IMPASTO_COMPUTE_BLUR", "IMPASTO_DARKEN_STEMS",
    "IMPASTO_DITHER", "IMPASTO_EARLY_Z", "IMPASTO_GRADIENT_LUT",
    "IMPASTO_IMAGE_CACHE_MB", "IMPASTO_IMAGE_UPLOAD", "IMPASTO_LCD_RGB", "IMPASTO_PAINT_UNIFORM",
    "IMPASTO_PREWARM", "IMPASTO_RETAIN_OFFSCREEN", "IMPASTO_SHADOW_2D",
    "IMPASTO_SHADOW_HOLLOW", "IMPASTO_SPECIALIZE_PAINT", "IMPASTO_PARALLEL_PREPARE",
    "IMPASTO_LATE_ACQUIRE", "IMPASTO_RENDER_GRAPH", "IMPASTO_SPATIAL_BATCH", "IMPASTO_STATE_TABLE",
    "IMPASTO_TEXT_GAMMA", "IMPASTO_TEXT_MSDF", "IMPASTO_TRANSIENT_ATTACHMENTS",
}


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


def run(checkout, prepared, backend, gpu, test_filter, experiments):
    binary, sha = prepared
    env = {
        **os.environ,
        "IMPASTO_BENCH_GIT_SHA": sha,
        "IMPASTO_BENCH_BACKEND": backend,
        "IMPASTO_BENCH_GPU": "1" if gpu else "0",
        **experiments,
    }
    output = capture([
        str(binary), test_filter, "--ignored", "--nocapture", "--test-threads=1",
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
    if not reports:
        raise ValueError(f"no benchmarks matched {test_filter} in {checkout}")
    if test_filter in SUITES and reports.keys() != SUITES[test_filter]:
        raise ValueError(f"{checkout} has unexpected benchmark scenes {sorted(reports)}")
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
    scenes = pairs[0]["base"].keys()
    if any(reports.keys() != scenes for pair in pairs for reports in (pair["base"], pair["head"])):
        raise ValueError("benchmark scene sets differ")
    for scene in sorted(scenes):
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


def configuration(argument):
    key, equals, value = argument.partition("=")
    if not equals or key not in EXPERIMENTS:
        raise argparse.ArgumentTypeError("expected an IMPASTO raster experiment as KEY=VALUE")
    return key, value


def counter_limit(argument):
    selector, equals, value = argument.partition("=")
    scene, slash, path = selector.partition("/")
    fields = path.split(".")
    try:
        maximum = int(value)
    except ValueError:
        maximum = -1
    if not (equals and slash and scene and all(fields) and maximum >= 0):
        raise argparse.ArgumentTypeError("expected SCENE/FIELD=MAX, e.g. hetero_cards_50/counters.draws=8")
    return scene, fields, maximum


def check_counters(pairs, limits, warmup):
    for scene, fields, maximum in limits:
        for round_number, pair in enumerate(pairs, start=1):
            report = pair["head"].get(scene)
            if report is None or len(report["samples"]) <= warmup:
                raise ValueError(f"{scene}: no samples after {warmup} warm-up frames")
            for sample in report["samples"][warmup:]:
                value = sample
                for key in fields:
                    value = value.get(key) if isinstance(value, dict) else None
                field = ".".join(fields)
                if type(value) is not int:
                    raise ValueError(f"{scene}: integer counter {field} is missing")
                if value > maximum:
                    raise ValueError(
                        f"{scene} round {round_number} frame {sample['frame']}: "
                        f"{field} is {value}, limit is {maximum}"
                    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path, help="checkout containing the baseline")
    parser.add_argument("head", type=Path, help="checkout containing the candidate")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--backend", choices=["auto", "metal", "vulkan", "gles", "dx12"], default="auto")
    parser.add_argument("--cpu-only", action="store_true")
    parser.add_argument("--filter", default="tests::p0::bench_", help="Rust libtest filter")
    parser.add_argument("--base-env", action="append", type=configuration, default=[], metavar="KEY=VALUE")
    parser.add_argument("--head-env", action="append", type=configuration, default=[], metavar="KEY=VALUE")
    parser.add_argument("--limit-counter", action="append", type=counter_limit, default=[],
                        metavar="SCENE/FIELD=MAX", help="fail on a candidate warm-frame counter")
    parser.add_argument("--require-warm-resources", action="store_true",
                        help="require zero buffer, bind group and synchronous pipeline creation on warm frames")
    parser.add_argument("--warmup-frames", type=int, default=3, help="initial frames skipped by counter gates")
    parser.add_argument("--out", type=Path, required=True, help="JSON report path")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    if args.warmup_frames < 0:
        parser.error("--warmup-frames must be nonnegative")
    checkouts = {"base": args.base.resolve(), "head": args.head.resolve()}
    shared = {key: os.environ[key] for key in EXPERIMENTS if key in os.environ}
    experiments = {
        "base": {**shared, **dict(args.base_env)},
        "head": {**shared, **dict(args.head_env)},
    }
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
            pair[side] = run(
                path, builds[path], args.backend, not args.cpu_only, args.filter, experiments[side],
            )
        pairs.append(pair)
    summary = compare(pairs)
    report = {
        "schema": 1,
        "host": {"platform": platform.platform(), "processor": platform.processor()},
        "experiments": experiments,
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
    limits = list(args.limit_counter)
    if args.require_warm_resources:
        limits.extend((scene, ["counters", counter], 0)
                      for scene in summary
                      for counter in ("buffers_created", "bind_groups_created", "synchronous_pipeline_compiles"))
    check_counters(pairs, limits, args.warmup_frames)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
