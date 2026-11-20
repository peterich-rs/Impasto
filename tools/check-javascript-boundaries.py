#!/usr/bin/env python3
"""Check that the host contracts and standalone runtime build independently."""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/javascript/src"
UI_CRATES = {"session", "uicore", "reactive", "engine", "shell", "macos", "android", "protocol"}
UI_IMPORT = re.compile(r"\b(?:(?:crate|super)::ui|" + "|".join(sorted(UI_CRATES)) + r")::")


def dependencies(package, *features):
    output = subprocess.check_output(
        [
            "cargo", "tree", "--locked", "-p", package, "--no-default-features", *features,
            "--edges", "normal,build", "--prefix", "none", "--format", "{p}",
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
    if forbidden := runtime_deps & UI_CRATES:
        raise SystemExit(f"standalone JavaScript runtime reaches UI crates: {sorted(forbidden)}")

    session_deps = dependencies("session")
    if "quickjs-jit" in session_deps or "reactive" in session_deps:
        raise SystemExit("the session contract pulls in the VM or reactive graph")

    for module in ("host", "compiler", "runtime", "standard"):
        for path in (SOURCE / module).rglob("*.rs"):
            if match := UI_IMPORT.search(path.read_text()):
                raise SystemExit(f"{path.relative_to(ROOT)} contains {match[0]}")

    print("Host contracts are std-only. The standalone runtime and session dependencies are isolated.")


if __name__ == "__main__":
    main()
