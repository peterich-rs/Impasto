#!/usr/bin/env python3
"""Fail unless the js crate's normal and build closure stays off the UI crates.

Dev-dependencies are ignored. rquickjs and js-host are allowed. A direct source
scan rejects UI crate paths inside crates/js; it does not replace the graph walk.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN = {
    "session",
    "uicore",
    "reactive",
    "engine",
    "shell",
    "quickjs",
    "macos",
    "android",
    "protocol",
}
SOURCE_MARKERS = (
    "session::",
    "uicore::",
    "reactive::",
    "engine::",
    "shell::",
    "quickjs::",
)


def metadata() -> dict:
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
            str(ROOT / "Cargo.toml"),
        ],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stderr)
        raise SystemExit(completed.returncode)
    return json.loads(completed.stdout)


def is_prod(kind: str | None) -> bool:
    return kind in (None, "build")


def prod_deps(node: dict) -> list[str]:
    found = []
    for dep in node.get("deps", []):
        kinds = dep.get("dep_kinds") or []
        if not kinds or any(is_prod(item.get("kind")) for item in kinds):
            found.append(dep["pkg"])
    return found


def main() -> None:
    graph = metadata()
    packages = {package["id"]: package for package in graph["packages"]}
    nodes = {node["id"]: node for node in graph["resolve"]["nodes"]}
    targets = [package_id for package_id, package in packages.items() if package["name"] == "js"]
    if len(targets) != 1:
        raise SystemExit(f"expected one js package, found {len(targets)}")
    target = targets[0]
    seen: set[str] = set()
    pending = prod_deps(nodes[target])
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        pending.extend(prod_deps(nodes[package_id]))
    names = sorted(packages[package_id]["name"] for package_id in seen)
    forbidden = sorted(set(names) & FORBIDDEN)
    if forbidden:
        raise SystemExit("js normal/build closure reaches UI crates: " + ", ".join(forbidden))
    root = ROOT / "crates" / "js" / "src"
    for path in root.rglob("*"):
        if path.suffix not in {".rs", ".js"}:
            continue
        text = path.read_text()
        for marker in SOURCE_MARKERS:
            if marker in text:
                raise SystemExit(f"{path} contains {marker}")
    print("js normal/build closure stays off the UI crates")


if __name__ == "__main__":
    main()
