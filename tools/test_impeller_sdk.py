#!/usr/bin/env python3
"""Unit tests for Impeller SDK target selection and matrix generation."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOLS_DIR = Path(__file__).resolve().parent
if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import impeller_sdk  # noqa: E402

ALL_TARGETS = list(impeller_sdk.TARGETS)
ANDROID_TARGETS = [
    "aarch64-linux-android",
    "x86_64-linux-android",
]
DARWIN_TARGETS = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]
IOS_TARGETS = [
    "aarch64-apple-ios",
    "aarch64-apple-ios-sim",
    "x86_64-apple-ios",
]
LINUX_TARGETS = [
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
]
WINDOWS_TARGETS = [
    "aarch64-pc-windows-msvc",
    "x86_64-pc-windows-msvc",
]
NON_MSVC_TARGETS = [target for target in ALL_TARGETS if not target.endswith("-msvc")]

BASE_LOCK = """\
version impeller-sdk-v1
source 77f043bc63196b9979cc09c5b97bb6d9b00be34d
repository peterich-rs/Impasto
artifact aarch64-apple-ios 0000000000000000000000000000000000000000000000000000000000000000 prebuilt/flutter-77f043bc6319/impeller-aarch64-apple-ios.zip
"""
ARTIFACT_ONLY_LOCK = """\
version impeller-sdk-v1
source 77f043bc63196b9979cc09c5b97bb6d9b00be34d
repository peterich-rs/Impasto
artifact aarch64-apple-ios 1111111111111111111111111111111111111111111111111111111111111111 prebuilt/flutter-77f043bc6319/impeller-aarch64-apple-ios.zip
"""
SOURCE_BUMP_LOCK = """\
version impeller-sdk-v1
source aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
repository peterich-rs/Impasto
"""
VERSION_BUMP_LOCK = """\
version impeller-sdk-v2
source 77f043bc63196b9979cc09c5b97bb6d9b00be34d
repository peterich-rs/Impasto
"""


class ParseTargetSpecTest(unittest.TestCase):
    def test_all_and_families(self):
        self.assertEqual(impeller_sdk.parse_target_spec("all"), ALL_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("darwin"), DARWIN_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("macos"), DARWIN_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("ios"), IOS_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("android"), ANDROID_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("linux"), LINUX_TARGETS)
        self.assertEqual(impeller_sdk.parse_target_spec("windows"), WINDOWS_TARGETS)

    def test_mixed_triples_dedup_and_canonical_order(self):
        spec = " x86_64-pc-windows-msvc , ios , aarch64-apple-ios , macos "
        self.assertEqual(
            impeller_sdk.parse_target_spec(spec),
            [*DARWIN_TARGETS, *IOS_TARGETS, "x86_64-pc-windows-msvc"],
        )

    def test_invalid_specs(self):
        for invalid in ("", "   ", "ios,", ",ios", "all,ios", "ios,all", "all,all", "unknown"):
            with self.subTest(spec=invalid):
                with self.assertRaises(ValueError):
                    impeller_sdk.parse_target_spec(invalid)


class SelectTargetsForChangesTest(unittest.TestCase):
    def test_non_source_build_paths_select_nothing(self):
        ignored = [
            ".github/workflows/impeller-sdk.yml",
            "crates/impeller-sys/README.md",
            "crates/impeller-sys/Cargo.toml",
            "crates/impeller-sys/build.rs",
            "crates/impeller-sys/src/lib.rs",
            "crates/impeller-sys/prebuilt/flutter-77f043bc6319/impeller-aarch64-apple-ios.zip",
            "crates/engine/src/lib.rs",
            "README.md",
        ]
        self.assertEqual(impeller_sdk.select_targets_for_changes(ignored), [])

    def test_sdk_lock_changes(self):
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], BASE_LOCK, ARTIFACT_ONLY_LOCK
            ),
            [],
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], BASE_LOCK, SOURCE_BUMP_LOCK
            ),
            ALL_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], BASE_LOCK, VERSION_BUMP_LOCK
            ),
            ALL_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], None, BASE_LOCK
            ),
            ALL_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], "broken lock", BASE_LOCK
            ),
            ALL_TARGETS,
        )
        with self.assertRaises(ValueError):
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], BASE_LOCK, None
            )
        with self.assertRaises(ValueError):
            impeller_sdk.select_targets_for_changes(
                [impeller_sdk.LOCK_RELPATH], BASE_LOCK, "broken lock"
            )

    def test_patch_scoping(self):
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                ["crates/impeller-sys/patches/android-vulkan-shutdown.patch"]
            ),
            ANDROID_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                ["crates/impeller-sys/patches/ios-interop.patch"]
            ),
            IOS_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                ["crates/impeller-sys/patches/windows-modf.patch"]
            ),
            WINDOWS_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes([
                "crates/impeller-sys/patches/ios-interop.patch",
                "crates/impeller-sys/patches/android-vulkan-shutdown.patch",
            ]),
            [*ANDROID_TARGETS, *IOS_TARGETS],
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(
                ["crates/impeller-sys/patches/new-cross-platform.patch"]
            ),
            ALL_TARGETS,
        )

    def test_bindings_headers_and_scripts(self):
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(["crates/impeller-sys/src/bindings.rs"]),
            NON_MSVC_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(["crates/impeller-sys/src/bindings_msvc.rs"]),
            WINDOWS_TARGETS,
        )
        self.assertEqual(
            impeller_sdk.select_targets_for_changes([
                "crates/impeller-sys/src/bindings.rs",
                "crates/impeller-sys/src/bindings_msvc.rs",
            ]),
            ALL_TARGETS,
        )
        for full_path in (
            "crates/impeller-sys/include/impeller.h",
            "crates/impeller-sys/include/LICENSE",
            "tools/build_impeller.py",
            "tools/impeller_sdk.py",
        ):
            with self.subTest(path=full_path):
                self.assertEqual(impeller_sdk.select_targets_for_changes([full_path]), ALL_TARGETS)

    def test_mixed_pr_only_builds_affected_platform(self):
        changed = [
            "crates/impeller-sys/patches/ios-interop.patch",
            "crates/impeller-sys/prebuilt/flutter-77f043bc6319/impeller-aarch64-apple-ios.zip",
            "crates/impeller-sys/sdk.lock",
            "crates/impeller-sys/src/lib.rs",
        ]
        self.assertEqual(
            impeller_sdk.select_targets_for_changes(changed, BASE_LOCK, ARTIFACT_ONLY_LOCK),
            IOS_TARGETS,
        )


class GitIntegrationTest(unittest.TestCase):
    def git(self, repo, *args):
        return subprocess.run(
            ["git", "-c", "user.name=Test", "-c", "user.email=test@example.com", *args],
            cwd=repo,
            check=True,
            capture_output=True,
            text=True,
        )

    def write_and_commit(self, repo, files, message):
        for relpath, content in files.items():
            path = repo / relpath
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        self.git(repo, "add", *files.keys())
        self.git(repo, "commit", "-m", message)

    def select_from_diff(self, repo, base, head="HEAD"):
        changed = impeller_sdk.changed_files_between(base, head, root=repo)
        base_lock = None
        head_lock = None
        if impeller_sdk.LOCK_RELPATH in changed:
            base_lock = impeller_sdk.read_git_blob(base, impeller_sdk.LOCK_RELPATH, root=repo)
            head_lock = impeller_sdk.read_git_blob(head, impeller_sdk.LOCK_RELPATH, root=repo)
        return impeller_sdk.select_targets_for_changes(changed, base_lock, head_lock)

    def test_merge_commit_and_platform_diffs(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            self.git(repo, "init", "-b", "main")
            self.write_and_commit(
                repo,
                {
                    impeller_sdk.LOCK_RELPATH: BASE_LOCK,
                    "crates/impeller-sys/patches/windows-modf.patch": "patch v1\n",
                    "crates/impeller-sys/README.md": "docs v1\n",
                },
                "base",
            )

            self.git(repo, "checkout", "-b", "pr-artifact")
            self.write_and_commit(
                repo,
                {
                    impeller_sdk.LOCK_RELPATH: ARTIFACT_ONLY_LOCK,
                    "crates/impeller-sys/README.md": "docs v2\n",
                },
                "artifact only",
            )
            self.git(repo, "checkout", "main")
            self.git(repo, "merge", "--no-ff", "pr-artifact", "-m", "merge artifact")
            self.assertEqual(self.select_from_diff(repo, "HEAD^1", "HEAD"), [])

            self.write_and_commit(
                repo,
                {"crates/impeller-sys/patches/windows-modf.patch": "patch v2\n"},
                "windows patch",
            )
            self.assertEqual(self.select_from_diff(repo, "HEAD^1", "HEAD"), WINDOWS_TARGETS)

            self.write_and_commit(
                repo,
                {impeller_sdk.LOCK_RELPATH: SOURCE_BUMP_LOCK},
                "bump source",
            )
            self.assertEqual(self.select_from_diff(repo, "HEAD^1", "HEAD"), ALL_TARGETS)


class CliMatrixTest(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, str(TOOLS_DIR / "impeller_sdk.py"), "matrix", *args],
            capture_output=True,
            text=True,
        )

    def test_matrix_targets_and_validation(self):
        full = self.run_cli()
        self.assertEqual(full.returncode, 0, full.stderr)
        self.assertEqual([item["target"] for item in json.loads(full.stdout)], ALL_TARGETS)

        ios = self.run_cli("--targets", "ios")
        self.assertEqual(ios.returncode, 0, ios.stderr)
        self.assertEqual([item["target"] for item in json.loads(ios.stdout)], IOS_TARGETS)

        combo = self.run_cli("--targets", "macos,android")
        self.assertEqual(combo.returncode, 0, combo.stderr)
        self.assertEqual(
            [item["target"] for item in json.loads(combo.stdout)],
            [*DARWIN_TARGETS, *ANDROID_TARGETS],
        )

        partial_publish = self.run_cli("--targets", "ios", "--require-all")
        self.assertNotEqual(partial_publish.returncode, 0)
        self.assertIn("requires all Impeller targets", partial_publish.stderr)

        orphan_head = self.run_cli("--head", "HEAD")
        self.assertNotEqual(orphan_head.returncode, 0)
        self.assertIn("--head requires --base", orphan_head.stderr)


if __name__ == "__main__":
    unittest.main()
