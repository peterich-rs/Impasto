#!/usr/bin/env python3
"""Build the pinned standalone Impeller library with Flutter's build tools."""

from pathlib import Path
import os
import platform
import runpy
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "build/impeller-source/flutter"
SDK = ROOT / "third_party/impeller-build"
REPOSITORY = "https://github.com/flutter/flutter.git"
INTEROP = "flutter/impeller/toolkit/interop"
PATCHES = {
    "ios": ("ios-interop.patch", f"{INTEROP}/impeller.cc"),
    "windows": ("windows-modf.patch", "flutter/impeller/entity/geometry/line_geometry.cc"),
}
TARGETS = {
    "darwin-arm64": ("Darwin", ("--mac", "--mac-cpu=arm64")),
    "darwin-x64": ("Darwin", ("--mac", "--mac-cpu=x64")),
    "ios-arm64": ("Darwin", ("--ios",)),
    "ios-arm64-sim": ("Darwin", ("--ios", "--simulator", "--simulator-cpu=arm64")),
    "ios-x64-sim": ("Darwin", ("--ios", "--simulator", "--simulator-cpu=x64")),
    "android-arm64": (None, ("--android", "--android-cpu=arm64")),
    "android-x64": (None, ("--android", "--android-cpu=x64")),
    "linux-arm64": ("Linux", ("--target-os=linux", "--linux-cpu=arm64")),
    # Flutter's native x64 host mode supplies the standalone ANGLE options.
    "linux-x64": ("Linux", ()),
    "windows-arm64": ("Windows", ("--windows-cpu=arm64",)),
    "windows-x64": ("Windows", ("--windows-cpu=x64",)),
}


def host_target():
    system = {"Darwin": "darwin", "Linux": "linux", "Windows": "windows"}.get(platform.system())
    cpu = {"aarch64": "arm64", "arm64": "arm64", "x86_64": "x64", "amd64": "x64"}.get(
        platform.machine().lower()
    )
    return f"{system}-{cpu}"


def run(*command, cwd):
    try:
        subprocess.run(command, cwd=cwd, check=True, stdout=sys.stderr)
    except FileNotFoundError as error:
        raise RuntimeError(f"could not run {command[0]}: {error}") from error


def prepare_windows_git():
    if shutil.which("git.bat"):
        return
    if not shutil.which("git.exe"):
        raise RuntimeError("git.exe is required on PATH to build Impeller on Windows")
    tools = SOURCE.parent / "windows-tools"
    tools.mkdir(parents=True, exist_ok=True)
    (tools / "git.bat").write_text("@git.exe %*\n", encoding="ascii")
    os.environ["PATH"] = f"{tools}{os.pathsep}{os.environ['PATH']}"


def gclient_config(template, android):
    solutions = runpy.run_path(str(template))["solutions"]
    overrides = solutions[0].setdefault("custom_vars", {})
    overrides["setup_githooks"] = False
    if not android:
        overrides["download_android_deps"] = False
    return f"solutions = {solutions!r}\n"


def prepare_source(revision, android):
    if not (SOURCE / ".git").exists():
        SOURCE.mkdir(parents=True, exist_ok=True)
        run("git", "init", cwd=SOURCE)
        run("git", "remote", "add", "origin", REPOSITORY, cwd=SOURCE)
    head = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"], cwd=SOURCE,
        capture_output=True, text=True,
    )
    if head.stdout.strip() != revision:
        run("git", "fetch", "--depth=1", REPOSITORY, revision, cwd=SOURCE)
        run("git", "checkout", "--detach", "FETCH_HEAD", cwd=SOURCE)

    synced = SOURCE.parent / "synced-revision"
    scope = "android" if android else "desktop"
    if synced.is_file() and synced.read_text().splitlines() in ([revision, scope], [revision, "android"]):
        return
    gclient = shutil.which("gclient")
    if not gclient:
        raise RuntimeError("gclient is required on PATH (from Chromium depot_tools)")
    template = SOURCE / "engine/scripts/standard.gclient"
    (SOURCE / ".gclient").write_text(gclient_config(template, android))
    run(gclient, "sync", "--no-history", cwd=SOURCE)
    synced.write_text(f"{revision}\n{scope}\n")


def library_paths(target):
    system = target.split("-")[0]
    if system == "ios":
        return [f"obj/{INTEROP}/libimpeller.a"]
    if system == "darwin":
        return ["libimpeller.dylib"]
    if system == "windows":
        return ["impeller.dll", "impeller.dll.lib"]
    if system == "android":
        return ["lib.stripped/libimpeller.so"]
    return ["libimpeller.so"]


def build(target, revision):
    host, args = TARGETS[target]
    if host and host != platform.system():
        raise RuntimeError(f"building {target} requires a {host} host")
    if target.startswith("android-") and host_target() not in {"darwin-arm64", "darwin-x64", "linux-x64"}:
        raise RuntimeError("building Android Impeller requires a macOS or Linux x64 host")

    output = SDK / target
    library_paths_for_target = library_paths(target)
    manifest = output / "source-revision"
    if (
        manifest.is_file()
        and manifest.read_text().splitlines()[:1] == [revision]
        and (output / "include/impeller.h").is_file()
        and (output / "LICENSE").is_file()
        and all((output / "lib" / Path(path).name).is_file() for path in library_paths_for_target)
    ):
        if not target.startswith("ios-") or (output / "link.txt").is_file():
            return output

    if host == "Windows":
        # Flutter's GN scripts invoke git.bat even when Git ships only git.exe.
        prepare_windows_git()

    if target.startswith(("darwin-", "ios-")):
        metal = subprocess.run(["xcrun", "metal", "--version"], stdout=subprocess.DEVNULL)
        if metal.returncode != 0:
            raise RuntimeError("install the Xcode Metal Toolchain with xcodebuild -downloadComponent metalToolchain")

    prepare_source(revision, target.startswith("android-"))
    engine_src = SOURCE / "engine/src"
    build_dir = engine_src / "out" / f"impasto-{target}"
    flutter_gn = runpy.run_path(str(engine_src / "flutter/tools/gn"))
    options = flutter_gn["parse_args"]([
        "gn", "--runtime-mode=release", "--no-enable-unittests", "--no-lto", *args,
    ])
    flutter_gn["validate_args"](options)
    gn_args = flutter_gn["to_command_line"](flutter_gn["to_gn_args"](options))
    suffix = ".exe" if platform.system() == "Windows" else ""
    pinned_gn = engine_src / "flutter/third_party/gn" / f"gn{suffix}"
    pinned_ninja = SOURCE / "third_party/ninja" / f"ninja{suffix}"
    target_type = "static_library" if target.startswith("ios-") else "shared_library"
    label = f"{INTEROP}:{target_type}"
    # Generate the standalone graph directly. The default iOS graph omits it.
    root_target = f"--root-target=//{label}"
    target_patch = PATCHES.get(target.split("-", 1)[0])
    patched_source = None
    if target_patch:
        patch_name, source_path = target_patch
        patched_source = engine_src / source_path
        original = patched_source.read_bytes()
        patch = ROOT / "crates/impeller-sys/patches" / patch_name
        run("git", "apply", "--ignore-space-change", str(patch), cwd=SOURCE)
    try:
        run(str(pinned_gn), "gen", str(build_dir), root_target, f"--args={' '.join(gn_args)}", cwd=engine_src)
        run(str(pinned_ninja), "-C", str(build_dir), label, cwd=engine_src)
    finally:
        if patched_source is not None:
            patched_source.write_bytes(original)

    SDK.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SDK) as temporary:
        staged = Path(temporary)
        (staged / "include").mkdir()
        (staged / "lib").mkdir()
        shutil.copy2(engine_src / INTEROP / "impeller.h", staged / "include/impeller.h")
        shutil.copy2(engine_src / "flutter/sky/packages/sky_engine/LICENSE", staged / "LICENSE")
        for path in library_paths_for_target:
            library = staged / "lib" / Path(path).name
            if target.startswith("ios-"):
                run("xcrun", "strip", "-S", "-o", str(library), str(build_dir / path), cwd=staged)
            else:
                shutil.copy2(build_dir / path, library)
        if target.startswith("darwin-"):
            library = staged / "lib/libimpeller.dylib"
            run("install_name_tool", "-id", "@rpath/libimpeller.dylib", str(library), cwd=staged)
            run("codesign", "--force", "--sign", "-", str(library), cwd=staged)
        if target.startswith("ios-"):
            describe = [pinned_gn, "desc", build_dir, f"//{label}"]
            frameworks = subprocess.check_output(
                [*describe, "frameworks", root_target], cwd=engine_src, text=True,
            ).split()
            libraries = subprocess.check_output(
                [*describe, "libs", root_target], cwd=engine_src, text=True,
            ).split()
            links = [f"framework={name.removesuffix('.framework')}" for name in frameworks]
            (staged / "link.txt").write_text("\n".join([*links, *libraries]) + "\n")

        output.mkdir(exist_ok=True)
        manifest.unlink(missing_ok=True)
        for directory in ("include", "lib"):
            (output / directory).mkdir(exist_ok=True)
            for file in (staged / directory).iterdir():
                file.replace(output / directory / file.name)
        (staged / "LICENSE").replace(output / "LICENSE")
        if (staged / "link.txt").is_file():
            (staged / "link.txt").replace(output / "link.txt")
        (staged / "source-revision").write_text(revision + "\n")
        (staged / "source-revision").replace(manifest)
    return output
