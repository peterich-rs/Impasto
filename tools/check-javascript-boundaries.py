#!/usr/bin/env python3
"""Check that the host contracts and standalone runtime build independently."""

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

    for module in ("host", "compiler", "runtime", "standard"):
        for path in (SOURCE / module).rglob("*.rs"):
            if match := UI_IMPORT.search(path.read_text()):
                raise SystemExit(f"{path.relative_to(ROOT)} contains {match[0]}")

    print("JavaScript host, standalone runtime, engine and raster dependency boundaries passed.")


if __name__ == "__main__":
    main()
