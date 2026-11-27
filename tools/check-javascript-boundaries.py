#!/usr/bin/env python3
"""Check that the host contracts and standalone runtime build independently."""

import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/javascript/src"
UI_CRATES = {"engine", "raster", "embedder"}
RASTER_PREFIXES = ("wgpu", "naga", "parley", "fontique", "swash", "lyon", "etagere")
UI_IMPORT = re.compile(
    r"\b(?:(?:crate|super)::(?:ui|js|session)|embedder_\w+|" + "|".join(sorted(UI_CRATES)) + r")::"
)


# Surface configuration is one diagnostics group with separate inputs.
RASTER_SWITCHES = {
    "IMPASTO_PREWARM",
    "IMPASTO_LATE_ACQUIRE",
    "IMPASTO_PARALLEL_PREPARE",
    "IMPASTO_SURFACE_COLOR",
    "IMPASTO_SURFACE_LATENCY",
    "IMPASTO_PRESENT_MODE",
    "IMPASTO_GPU_VALIDATE",
}
RASTER_ENV = re.compile(r"""["'](IMPASTO_[A-Z0-9_]+)["']""")
DISTANCE_FIELD_CRATES = {"fdsm", "msdfgen", "msdfgen-sys", "msdfgen-rs"}
STATIC_TOPOLOGY = re.compile(r"\b(?:ThreadTopology|IMPASTO_THREAD_TOPOLOGY|impastoThreadTopology)\b|app\.thread_topology")


def check_raster_architecture(raster_deps):
    if forbidden := DISTANCE_FIELD_CRATES & raster_deps:
        raise SystemExit(f"raster reaches runtime distance-field generation: {sorted(forbidden)}")

    for path in (ROOT / "crates/raster/src").rglob("*.rs"):
        if path.name == "tests.rs" or "tests" in path.parts:
            continue
        if forbidden := set(RASTER_ENV.findall(path.read_text())) - RASTER_SWITCHES:
            raise SystemExit(f"{path.relative_to(ROOT)} adds raster environment switches: {sorted(forbidden)}")

    android_projects = (ROOT / "examples").glob("*/android/app/build.gradle.kts")
    for path in [*(ROOT / "crates/embedder").rglob("*.rs"),
                 *(ROOT / "crates/embedder").rglob("*.gradle.kts"),
                 *android_projects]:
        if match := STATIC_TOPOLOGY.search(path.read_text()):
            raise SystemExit(f"{path.relative_to(ROOT)} reintroduces static thread placement: {match[0]}")


def dependencies(package, *features):
    output = subprocess.check_output(
        [
            "cargo", "tree", "--locked", "-p", package, "--no-default-features", *features,
            "--target", "all", "--edges", "normal,build", "--prefix", "none", "--format", "{p}",
        ],
        cwd=ROOT,
        text=True,
    )
    return {line.split()[0] for line in output.splitlines() if line.strip()} - {package}


def main():
    host_deps = dependencies("javascript")
    if host_deps:
        raise SystemExit(f"host contracts must stay std-only: {sorted(host_deps)}")

    runtime_deps = dependencies("javascript", "--features", "runtime")
    if forbidden := {
        name for name in runtime_deps if name in UI_CRATES or name.startswith(("embedder-", *RASTER_PREFIXES))
    }:
        raise SystemExit(f"standalone JavaScript runtime reaches UI crates: {sorted(forbidden)}")

    engine_deps = dependencies("engine")
    raster_deps = dependencies("raster")
    check_raster_architecture(raster_deps)
    for package, package_deps in (("engine", engine_deps), ("raster", raster_deps)):
        if forbidden := {
            name for name in package_deps
            if name.startswith(("rquickjs", "quickjs-jit"))
        }:
            raise SystemExit(f"{package} reaches the JavaScript VM: {sorted(forbidden)}")

    if forbidden := {
        name for name in engine_deps
        if name in {"khronos-egl", "libloading", "jni"}
        or name.startswith(("ndk", "objc", "embedder", "windows", "android", *RASTER_PREFIXES))
        or name == "raster"
    }:
        raise SystemExit(f"engine reaches platform or raster crates: {sorted(forbidden)}")

    sdk_deps = dependencies("impasto")
    if forbidden := {
        name for name in sdk_deps if name in UI_CRATES
        or name.startswith(("embedder-", "rquickjs", "quickjs-jit", "tokio", *RASTER_PREFIXES))
    }:
        raise SystemExit(f"Impasto plugin SDK reaches its host implementation: {sorted(forbidden)}")

    metadata = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        cwd=ROOT,
        text=True,
    )
    crates = {Path(p["manifest_path"]): p["name"] for p in json.loads(metadata)["packages"]}
    for manifest in sorted((ROOT / "packages").glob("*/rust/Cargo.toml")):
        if manifest not in crates:
            raise SystemExit(f"{manifest.relative_to(ROOT)} is missing from the workspace")
        name = crates[manifest]
        if forbidden := {
            dep for dep in dependencies(name)
            if dep in UI_CRATES or dep.startswith(("embedder-", "rquickjs", "quickjs-jit", *RASTER_PREFIXES))
        }:
            raise SystemExit(f"{name} reaches VM, renderer or host crates: {sorted(forbidden)}")

    toolchain_crates = {"bundler", "testing"}
    runtime_crates = UI_CRATES | {"javascript", "impasto", "impasto-macros"}
    for package in sorted(set(crates.values())):
        if package in runtime_crates or package.startswith(("embedder-", "impasto-")):
            if forbidden := dependencies(package) & toolchain_crates:
                raise SystemExit(f"{package} reaches host-only tools: {sorted(forbidden)}")

    for module in ("host", "compiler", "runtime", "standard"):
        for path in (SOURCE / module).rglob("*.rs"):
            if match := UI_IMPORT.search(path.read_text()):
                raise SystemExit(f"{path.relative_to(ROOT)} contains {match[0]}")

    print("JavaScript host, standalone runtime, engine, raster, and plugin dependency boundaries passed.")


if __name__ == "__main__":
    main()
