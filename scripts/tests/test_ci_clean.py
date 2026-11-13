"""Exercise CI cleanup without touching real checkout or shared build files."""

import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

JUSTFILE = Path(__file__).resolve().parents[2] / ".github/Justfile"


@unittest.skipIf(sys.platform == "win32", "Exercises the Unix CI cleanup recipe")
class CiCleanTests(unittest.TestCase):
    def test_removes_checkout_targets_but_preserves_shared_target(self):
        self.check_cleanup()

    def test_refuses_local_cleanup(self):
        self.check_cleanup(actions="false")

    def test_refuses_another_workspace(self):
        self.check_cleanup(matching_workspace=False)

    def test_preserves_main_cache_inputs(self):
        self.check_cleanup(event="push")

    def test_refuses_a_target_linked_to_shared_files(self):
        self.check_cleanup(linked_target=True)

    def check_cleanup(self, *, actions="true", matching_workspace=True, event="pull_request", linked_target=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = (Path(temporary) / "checkout").resolve()
            ffi = root / "src/sdk/dotnet/ffi"
            ffi.mkdir(parents=True)
            for manifest in (root / "Cargo.toml", ffi / "Cargo.toml"):
                manifest.write_text("[workspace]\nmembers = []\n", encoding="utf-8")
            shared = root.parent / "shared-target"
            shared.mkdir()
            preserved = shared / "keep"
            preserved.write_text("shared", encoding="utf-8")
            targets = (root / "target", ffi / "target")
            for target in targets:
                if linked_target and target == targets[0]:
                    target.symlink_to(shared, target_is_directory=True)
                else:
                    target.mkdir()
                (target / "build-output").write_text("generated", encoding="utf-8")

            result = subprocess.run(
                ["just", "--justfile", str(JUSTFILE), "ci-clean"],
                cwd=root,
                env={
                    **os.environ,
                    "GITHUB_ACTIONS": actions,
                    "GITHUB_EVENT_NAME": event,
                    "RUNNER_OS": "macOS",
                    "GITHUB_WORKSPACE": str(root if matching_workspace else shared),
                    "CARGO_TARGET_DIR": str(shared),
                },
                capture_output=True,
                text=True,
                timeout=30,
            )

            allowed = actions == "true" and matching_workspace and event == "pull_request" and not linked_target
            if allowed:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(all(not target.exists() for target in targets))
            else:
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(all((target / "build-output").exists() for target in targets))
            self.assertEqual(preserved.read_text(encoding="utf-8"), "shared")
