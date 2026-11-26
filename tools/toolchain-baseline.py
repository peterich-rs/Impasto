#!/usr/bin/env python3
"""Measure Gallery ESM builds with the same native engine and frame scenarios."""

import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess

ROOT = Path(__file__).resolve().parent.parent
CURRENT = ROOT / "examples/gallery-solid/dist/quotes.js"
PATTERN = re.compile(r"BENCH (click|scroll) (\d+) (\d+) (\d+) (\d+) ([\d.]+)")
FIRST = re.compile(r"FIRST (click|scroll) (\d+) (\d+) (\d+) ([\d.]+)")
STARTUP = re.compile(r"STARTUP (click|scroll) ([\d.]+)")
IQJS = re.compile(r"IQJS (\d+)")
TEST = [
    "cargo", "test", "--locked", "-p", "engine", "--features", "js", "--lib",
    "toolchain_baseline_counts", "--", "--ignored", "--nocapture",
]


def measure(bundle):
    env = {**os.environ, "IMPASTO_BENCH_GALLERY": str(bundle)}
    result = subprocess.run(TEST, cwd=ROOT, env=env, text=True, capture_output=True)
    if result.returncode:
        raise SystemExit(result.stdout + result.stderr)
    samples = {
        scenario: {"updates": int(updates), "sets": int(sets), "ffi_calls": int(calls),
                   "ffi_batches": int(batches), "script_us": float(us)}
        for scenario, updates, sets, calls, batches, us in PATTERN.findall(result.stdout)
    }
    for scenario, nodes, calls, batches, us in FIRST.findall(result.stdout):
        samples[scenario]["first_nodes"] = int(nodes)
        samples[scenario]["first_ffi_calls"] = int(calls)
        samples[scenario]["first_ffi_batches"] = int(batches)
        samples[scenario]["first_script_us"] = float(us)
    for scenario, us in STARTUP.findall(result.stdout):
        samples[scenario]["startup_us"] = float(us)
    if (samples.keys() != {"click", "scroll"} or
            any(not {"first_ffi_calls", "startup_us"} <= sample.keys()
                for sample in samples.values())):
        raise SystemExit("The engine did not report both Gallery scenarios")
    bytecode = IQJS.search(result.stdout)
    if not bytecode:
        raise SystemExit("The engine did not report the IQJS bytecode size")
    samples["iqjs_bytes"] = int(bytecode.group(1))
    return samples


def summarize(bundle, samples):
    return {
        "esm_bytes": bundle.stat().st_size,
        "iqjs_bytes": samples[0]["iqjs_bytes"],
        "samples": samples,
        "median": {
            scenario: {
                key: statistics.median(sample[scenario][key] for sample in samples)
                for key in samples[0][scenario]
            }
            for scenario in ("click", "scroll")
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, help="Optional compatible baseline ESM")
    parser.add_argument("--rounds", type=int, default=10)
    parser.add_argument("--min-ffi-reduction", type=float, metavar="PERCENT",
                        help="Require this reduction for first frame, click, and scroll")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    if args.min_ffi_reduction is not None and (not args.before or not 0 <= args.min_ffi_reduction <= 100):
        parser.error("--min-ffi-reduction requires --before and a percentage between 0 and 100")
    bundles = {"after": CURRENT}
    if args.before:
        bundles = {"before": args.before.resolve(), **bundles}
    samples = {name: [] for name in bundles}
    for index in range(args.rounds):
        order = bundles if index % 2 == 0 else reversed(bundles)
        for name in order:
            samples[name].append(measure(bundles[name]))
    results = {name: summarize(path, samples[name]) for name, path in bundles.items()}
    output = ROOT / "target/toolchain-baseline/results.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps({name: result["median"] | {key: result[key] for key in ("esm_bytes", "iqjs_bytes")}
                      for name, result in results.items()}, indent=2))
    print(f"Raw samples: {output}")
    if args.min_ffi_reduction is not None:
        failures = []
        for label, scenario, metric in (("first frame", "click", "first_ffi_calls"),
                                        ("click", "click", "ffi_calls"), ("scroll", "scroll", "ffi_calls")):
            before = results["before"]["median"][scenario][metric]
            after = results["after"]["median"][scenario][metric]
            reduction = 100 * (before - after) / before if before else 0
            print(f"{label} FFI reduction: {reduction:.1f}%")
            if reduction < args.min_ffi_reduction:
                failures.append(label)
        if failures:
            raise SystemExit("FFI reduction below target: " + ", ".join(failures))


if __name__ == "__main__":
    main()
