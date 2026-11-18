#!/usr/bin/env python3
"""Fail unless js-host's normal and build dependency closure is empty.

Dev-dependencies are not part of that graph. The only workspace package that
may depend on js-host through a normal or build edge is session.
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
    "quickjs",
    "engine",
    "shell",
    "macos",
    "android",
    "reactive",
    "protocol",
    "rquickjs",
    "js",
}


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
    found: list[str] = []
    for dep in node.get("deps", []):
        kinds = dep.get("dep_kinds") or []
        if not kinds or any(is_prod(item.get("kind")) for item in kinds):
            found.append(dep["pkg"])
    return found


def main() -> None:
    graph = metadata()
    packages = {package["id"]: package for package in graph["packages"]}
    nodes = {node["id"]: node for node in graph["resolve"]["nodes"]}
    hosts = [package_id for package_id, package in packages.items() if package["name"] == "js-host"]
    if len(hosts) != 1:
        raise SystemExit(f"expected one js-host package, found {len(hosts)}")
    host_id = hosts[0]

    declared = [
        dep["name"]
        for dep in packages[host_id].get("dependencies", [])
        if is_prod(dep.get("kind"))
    ]
    if declared:
        raise SystemExit(f"js-host declares normal or build dependencies: {declared}")

    seen: set[str] = set()
    pending = prod_deps(nodes[host_id])
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        pending.extend(prod_deps(nodes[package_id]))
    if seen:
        names = sorted(packages[package_id]["name"] for package_id in seen)
        forbidden = sorted(set(names) & FORBIDDEN)
        raise SystemExit(
            "js-host normal/build closure is not std-only: "
            + ", ".join(names)
            + (f" (forbidden: {', '.join(forbidden)})" if forbidden else "")
        )

    dependents: list[str] = []
    for node_id, node in nodes.items():
        if node_id == host_id:
            continue
        if host_id in prod_deps(node):
            dependents.append(packages[node_id]["name"])
    dependents.sort()
    if dependents != ["session"]:
        raise SystemExit(
            "normal/build dependents of js-host must be exactly session, "
            f"found {dependents or 'none'}"
        )
    print("js-host normal/build closure is std-only; only session depends on it")


if __name__ == "__main__":
    main()
