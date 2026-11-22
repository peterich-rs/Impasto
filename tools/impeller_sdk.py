#!/usr/bin/env python3
"""Build, publish and prepare the Impasto Impeller SDK."""

import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.parse import urljoin
from urllib.request import urlopen
from zipfile import BadZipFile, ZIP_DEFLATED, ZipFile

import build_impeller


ROOT = build_impeller.ROOT
CRATE = ROOT / "crates/impeller-sys"
LOCK_FILE = CRATE / "sdk.lock"
HEADER = CRATE / "include/impeller.h"
CACHE = ROOT / "third_party/impeller-sdk"
BINDGEN_VERSION = "0.72.1"
TARGETS = {
    "aarch64-apple-darwin": ("darwin-arm64", "macos-15"),
    "x86_64-apple-darwin": ("darwin-x64", "macos-15-intel"),
    "aarch64-linux-android": ("android-arm64", "ubuntu-24.04"),
    "x86_64-linux-android": ("android-x64", "ubuntu-24.04"),
    "aarch64-apple-ios": ("ios-arm64", "macos-15"),
    "aarch64-apple-ios-sim": ("ios-arm64-sim", "macos-15"),
    "x86_64-apple-ios": ("ios-x64-sim", "macos-15"),
    "aarch64-unknown-linux-gnu": ("linux-arm64", "ubuntu-24.04-arm"),
    "x86_64-unknown-linux-gnu": ("linux-x64", "ubuntu-24.04"),
    "aarch64-pc-windows-msvc": ("windows-arm64", "windows-2025"),
    "x86_64-pc-windows-msvc": ("windows-x64", "windows-2025"),
}
SIMULATOR_CLANG = {
    "aarch64-apple-ios-sim": "aarch64-apple-ios-simulator",
    "x86_64-apple-ios": "x86_64-apple-ios-simulator",
}
LOCK_RELPATH = "crates/impeller-sys/sdk.lock"
FAMILY_ALIASES = {"macos": "darwin"}
FULL_BUILD_PATHS = {"tools/build_impeller.py", "tools/impeller_sdk.py"}


@dataclass(frozen=True)
class SdkLock:
    version: str
    source: str
    repository: str
    artifacts: dict

    @classmethod
    def parse(cls, text):
        fields = {}
        artifacts = {}
        for line in text.splitlines():
            parts = line.split()
            if not parts:
                continue
            if parts[0] == "artifact" and len(parts) == 4:
                _, target, digest, url = parts
                if target not in TARGETS or target in artifacts or not re.fullmatch(r"[0-9a-f]{64}", digest):
                    raise ValueError(f"invalid SDK artifact for {target}")
                artifacts[target] = (digest, url)
            elif len(parts) == 2 and parts[0] in {"version", "source", "repository"} and parts[0] not in fields:
                fields[parts[0]] = parts[1]
            else:
                raise ValueError("invalid entry in Impeller sdk.lock")

        version = fields.get("version", "")
        source = fields.get("source", "")
        repository = fields.get("repository", "")
        if not re.fullmatch(r"impeller-sdk-v[0-9]+", version):
            raise ValueError("invalid Impeller SDK version")
        if not re.fullmatch(r"[0-9a-f]{40}", source):
            raise ValueError("invalid Impeller source revision")
        if not re.fullmatch(r"[\w.-]+/[\w.-]+", repository):
            raise ValueError("invalid Impeller SDK repository")
        return cls(version, source, repository, artifacts)

    def identity(self):
        return self.version, self.source, self.repository

    def metadata(self, target):
        return f"{self.version}\n{self.source}\n{target}\n"

    def release_url(self):
        return f"https://github.com/{self.repository}/releases/download/{self.version}"

    def text(self):
        lines = [f"version {self.version}", f"source {self.source}", f"repository {self.repository}"]
        for target, (digest, url) in sorted(self.artifacts.items()):
            lines.append(f"artifact {target} {digest} {url}")
        return "\n".join(lines) + "\n"


def host_target():
    platform = build_impeller.host_target()
    for target, (candidate, _) in TARGETS.items():
        if candidate == platform:
            return target
    raise ValueError(f"unsupported Impeller SDK host {platform}")


def library_sources(target):
    platform, _ = TARGETS[target]
    for path in build_impeller.library_paths(platform):
        name = Path(path).name
        # Cargo's MSVC linker finds the DLL's import library via -l impeller.
        yield name, "impeller.lib" if name == "impeller.dll.lib" else name


def sdk_files(target):
    files = ["sdk.meta", "bindings.rs", "include/impeller.h", "LICENSE"]
    files.extend(f"lib/{name}" for _, name in library_sources(target))
    if "-apple-ios" in target:
        files.append("link.txt")
    return files


def installed(sdk, target, lock):
    marker = sdk / "sdk.meta"
    return (
        marker.is_file()
        and marker.read_text() == lock.metadata(target)
        and all((sdk / name).is_file() and (sdk / name).stat().st_size for name in sdk_files(target))
    )


@contextmanager
def exclusive(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as lock:
        if os.name == "nt":
            import msvcrt

            lock.seek(0)
            while True:
                try:
                    msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
                    break
                except OSError as error:
                    if error.errno != errno.EACCES:
                        raise
                    time.sleep(0.25)
        else:
            import fcntl

            fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def cache_path(cache, target, lock):
    return cache.resolve() / lock.version / target


def artifact_name(target):
    return f"impeller-{target}.zip"


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def replace_sdk(staged, output):
    backup = staged.parent / "previous"
    if output.exists():
        output.replace(backup)
    try:
        staged.replace(output)
    except OSError:
        if backup.exists():
            backup.replace(output)
        raise


def read_index(lock, override):
    if override:
        index = SdkLock.parse(Path(override).read_text())
    elif lock.artifacts:
        index = lock
    else:
        url = f"{lock.release_url()}/sdk.lock"
        with urlopen(url, timeout=30) as response:
            index = SdkLock.parse(response.read(65536).decode())
        for target, (_, url) in index.artifacts.items():
            if url != f"{lock.release_url()}/{artifact_name(target)}":
                raise ValueError(f"unexpected Release URL for {target}")
    if index.identity() != lock.identity():
        raise ValueError("Impeller SDK index does not match the workspace sdk.lock")
    return index


def prepare(cache, target, lock, override):
    sdk = cache_path(cache, target, lock)
    with exclusive(sdk.with_suffix(".lock")):
        expected_digest = lock.artifacts.get(target, (None, None))[0]
        fingerprint = sdk / "artifact.sha256"
        if installed(sdk, target, lock) and (
            override is None and expected_digest is None
            or override is None and fingerprint.is_file()
            and fingerprint.read_text().strip() == expected_digest
        ):
            return sdk
        try:
            index = read_index(lock, override)
            digest, url = index.artifacts[target]
        except (OSError, KeyError) as error:
            raise RuntimeError(
                f"SDK {lock.version} for {target} is unavailable ({error}). "
                f"Publish its CI package or run `just impeller-source {target}`."
            ) from error

        with tempfile.TemporaryDirectory(dir=sdk.parent) as temporary:
            work = Path(temporary)
            archive = work / "sdk.zip"
            index_path = Path(override) if override else LOCK_FILE
            url = urljoin(index_path.resolve().as_uri(), url)
            with urlopen(url, timeout=120) as response, archive.open("wb") as stream:
                shutil.copyfileobj(response, stream, 1024 * 1024)
            if sha256(archive) != digest:
                raise ValueError(f"Impeller SDK SHA-256 mismatch for {target}")

            staged = work / "extracted"
            expected = sdk_files(target)
            with ZipFile(archive) as bundle:
                if sorted(bundle.namelist()) != sorted(expected):
                    raise ValueError(f"unexpected contents in the Impeller SDK for {target}")
                for name in expected:
                    destination = staged / name
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.open(name) as source, destination.open("wb") as output:
                        shutil.copyfileobj(source, output, 1024 * 1024)
            if not installed(staged, target, lock):
                raise ValueError(f"Impeller SDK metadata does not match {target} and {lock.version}")
            (staged / "artifact.sha256").write_text(f"{digest}\n")
            replace_sdk(staged, sdk)
    return sdk


def generate_bindings(header, destination, target):
    tool = os.environ.get("IMPASTO_BINDGEN", "bindgen")
    version = subprocess.check_output([tool, "--version"], text=True).strip()
    if version != f"bindgen {BINDGEN_VERSION}":
        raise RuntimeError(f"Impeller requires bindgen-cli {BINDGEN_VERSION}, got {version}")
    text = header.read_text()
    abi = 0
    for field, shift in [("VARIANT", 29), ("MAJOR", 22), ("MINOR", 12), ("PATCH", 0)]:
        match = re.search(rf"^#define IMPELLER_VERSION_{field}\s+(\d+)$", text, re.MULTILINE)
        if not match:
            raise ValueError(f"missing IMPELLER_VERSION_{field} in the SDK header")
        abi |= int(match[1]) << shift

    subprocess.run([
        tool, str(header), "--output", str(destination),
        "--allowlist-function", "Impeller.*", "--allowlist-type", "Impeller.*",
        "--allowlist-var", "kImpeller.*", "--default-enum-style", "moduleconsts",
        "--no-doc-comments", "--no-layout-tests", "--rust-target", "1.85", "--rust-edition", "2024",
        "--raw-line", f"pub const IMPELLER_VERSION: u32 = {abi};",
        "--", "-ffreestanding", f"--target={SIMULATOR_CLANG.get(target, target)}",
    ], check=True, stdout=sys.stderr)


def bindings_path(target):
    name = "bindings_msvc.rs" if target.endswith("-msvc") else "bindings.rs"
    return CRATE / "src" / name


def refresh_bindings():
    for target in ("aarch64-apple-darwin", "x86_64-pc-windows-msvc"):
        generate_bindings(HEADER, bindings_path(target), target)


def build(cache, target, lock):
    platform, _ = TARGETS[target]
    sdk = cache_path(cache, target, lock)
    with exclusive(build_impeller.SOURCE.parent / "build.lock"):
        native = build_impeller.build(platform, lock.source)
        source_header = native / "include/impeller.h"
        if source_header.read_text() != HEADER.read_text():
            raise ValueError("the Impeller source header differs from crates/impeller-sys/include/impeller.h")
        with exclusive(sdk.with_suffix(".lock")):
            with tempfile.TemporaryDirectory(dir=sdk.parent) as temporary:
                staged = Path(temporary) / "sdk"
                (staged / "include").mkdir(parents=True)
                (staged / "lib").mkdir()
                header = staged / "include/impeller.h"
                shutil.copyfile(source_header, header)
                shutil.copyfile(native / "LICENSE", staged / "LICENSE")
                for original, name in library_sources(target):
                    shutil.copyfile(native / "lib" / original, staged / "lib" / name)
                if "-apple-ios" in target:
                    shutil.copyfile(native / "link.txt", staged / "link.txt")
                generated = staged / "bindings.rs"
                generate_bindings(header, generated, target)
                if generated.read_text() != bindings_path(target).read_text():
                    raise ValueError(f"bindings differ for {target}. Run `python3 tools/impeller_sdk.py bindings`")
                (staged / "sdk.meta").write_text(lock.metadata(target))
                if target in lock.artifacts:
                    (staged / "artifact.sha256").write_text(f"{lock.artifacts[target][0]}\n")
                replace_sdk(staged, sdk)
    return sdk


def package(cache, target, lock, directory):
    sdk = cache_path(cache, target, lock)
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / artifact_name(target)
    with exclusive(sdk.with_suffix(".lock")):
        if not installed(sdk, target, lock):
            raise ValueError(f"no prepared SDK for {target} at {sdk}")
        with tempfile.TemporaryDirectory(dir=directory) as temporary:
            staged = Path(temporary) / archive.name
            with ZipFile(staged, "w", ZIP_DEFLATED) as bundle:
                for name in sdk_files(target):
                    bundle.write(sdk / name, name)
            staged.replace(archive)
    return archive.resolve()


def write_index(directory, lock, release):
    artifacts = {}
    for target in TARGETS:
        path = (directory / artifact_name(target)).resolve()
        if not path.is_file():
            if release:
                raise ValueError(f"release requires the {target} SDK package")
            continue
        with ZipFile(path) as bundle:
            if sorted(bundle.namelist()) != sorted(sdk_files(target)):
                raise ValueError(f"unexpected contents in {path}")
            if bundle.read("sdk.meta").decode().splitlines() != lock.metadata(target).splitlines():
                raise ValueError(f"unexpected SDK version or target in {path}")
        url = f"{lock.release_url()}/{path.name}" if release else path.as_uri()
        artifacts[target] = (sha256(path), url)
    if not artifacts:
        raise ValueError(f"no Impeller packages in {directory}")
    index = directory / "sdk.lock"
    index.write_text(SdkLock(*lock.identity(), artifacts).text())
    return index.resolve()


def targets_for_family(family):
    canonical = FAMILY_ALIASES.get(family, family)
    return [
        target
        for target, (platform, _) in TARGETS.items()
        if platform.split("-", 1)[0] == canonical
    ]


def parse_target_spec(spec):
    if not spec.strip():
        raise ValueError("empty Impeller target selection")
    tokens = [token.strip() for token in spec.split(",")]
    if any(not token for token in tokens):
        raise ValueError("empty Impeller target selection")
    if tokens == ["all"]:
        return list(TARGETS)
    if "all" in tokens:
        raise ValueError("cannot combine 'all' with specific Impeller targets")

    selected = set()
    for token in tokens:
        if token in TARGETS:
            selected.add(token)
        elif family_targets := targets_for_family(token):
            selected.update(family_targets)
        else:
            raise ValueError(f"unknown Impeller target or platform family: {token}")
    return [target for target in TARGETS if target in selected]


def patch_target_families():
    return {
        f"crates/impeller-sys/patches/{patch_name}": family
        for family, (patch_name, _) in build_impeller.PATCHES.items()
    }


def select_targets_for_changes(changed_paths, base_lock_text=None, head_lock_text=None):
    patches = patch_target_families()
    selected = set()
    for path in changed_paths:
        if path in FULL_BUILD_PATHS or path.startswith("crates/impeller-sys/include/"):
            return list(TARGETS)
        if path == "crates/impeller-sys/src/bindings.rs":
            selected.update(target for target in TARGETS if not target.endswith("-msvc"))
        elif path == "crates/impeller-sys/src/bindings_msvc.rs":
            selected.update(target for target in TARGETS if target.endswith("-msvc"))
        elif path.startswith("crates/impeller-sys/patches/"):
            if path in patches:
                selected.update(targets_for_family(patches[path]))
            else:
                return list(TARGETS)
        elif path == LOCK_RELPATH:
            if head_lock_text is None:
                raise ValueError(f"missing {LOCK_RELPATH} at head revision")
            head_lock = SdkLock.parse(head_lock_text)
            if base_lock_text is None:
                return list(TARGETS)
            try:
                base_lock = SdkLock.parse(base_lock_text)
            except ValueError:
                return list(TARGETS)
            if (base_lock.version, base_lock.source) != (head_lock.version, head_lock.source):
                return list(TARGETS)
    return [target for target in TARGETS if target in selected]


def changed_files_between(base, head="HEAD", root=ROOT):
    result = subprocess.run(
        ["git", "-c", "core.quotepath=false", "diff", "-z", "--name-only", "--no-renames", base, head],
        cwd=root,
        check=True,
        capture_output=True,
    )
    return [entry for entry in result.stdout.decode("utf-8").split("\0") if entry]


def read_git_blob(rev, relpath, root=ROOT):
    result = subprocess.run(
        ["git", "show", f"{rev}:{relpath}"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    return result.stdout if result.returncode == 0 else None


def matrix_entries(targets):
    return [
        {"target": target, "runner": TARGETS[target][1], "platform": TARGETS[target][0]}
        for target in targets
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=Path(os.environ.get("IMPASTO_IMPELLER_CACHE", CACHE)))
    commands = parser.add_subparsers(dest="command", required=True)
    for action in ("prepare", "build", "package"):
        command = commands.add_parser(action)
        command.add_argument("target", nargs="?", choices=TARGETS)
        if action == "package":
            command.add_argument("--output", type=Path, required=True)
    index = commands.add_parser("index")
    index.add_argument("directory", type=Path)
    index.add_argument("--release", action="store_true")
    matrix = commands.add_parser("matrix")
    matrix_selector = matrix.add_mutually_exclusive_group()
    matrix_selector.add_argument("--targets")
    matrix_selector.add_argument("--base")
    matrix.add_argument("--head")
    matrix.add_argument("--require-all", action="store_true")
    commands.add_parser("bindings")
    args = parser.parse_args()
    try:
        lock = SdkLock.parse(LOCK_FILE.read_text())
        if args.command == "matrix":
            if args.head is not None and args.base is None:
                raise ValueError("--head requires --base")
            if args.base is not None:
                if not args.base:
                    raise ValueError("--base cannot be empty")
                head = args.head if args.head is not None else "HEAD"
                if not head:
                    raise ValueError("--head cannot be empty")
                changed = changed_files_between(args.base, head)
                base_lock_text = None
                head_lock_text = None
                if LOCK_RELPATH in changed:
                    base_lock_text = read_git_blob(args.base, LOCK_RELPATH)
                    head_lock_text = read_git_blob(head, LOCK_RELPATH)
                selected = select_targets_for_changes(changed, base_lock_text, head_lock_text)
            elif args.targets is not None:
                selected = parse_target_spec(args.targets)
            else:
                selected = list(TARGETS)
            if args.require_all and len(selected) != len(TARGETS):
                raise ValueError("publishing a release requires all Impeller targets (targets=all)")
            print(json.dumps(matrix_entries(selected)))
        elif args.command == "index":
            print(write_index(args.directory, lock, args.release))
        elif args.command == "bindings":
            refresh_bindings()
        else:
            target = args.target or host_target()
            if args.command == "build":
                print(build(args.cache_dir, target, lock))
            elif args.command == "package":
                print(package(args.cache_dir, target, lock, args.output))
            else:
                print(prepare(args.cache_dir, target, lock, os.environ.get("IMPASTO_IMPELLER_LOCK")))
    except (BadZipFile, OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Impeller SDK: {error}\n")


if __name__ == "__main__":
    main()
