"""Check that the Linux worker recipe builds an executable."""

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


@unittest.skipUnless(sys.platform == "linux", "Linux worker build regression")
class WorkerBuildTests(unittest.TestCase):
    def test_build_invokes_cargo_on_linux(self):
        repository = Path(__file__).resolve().parents[2]
        just = shutil.which("just")
        self.assertIsNotNone(just)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "bin"
            tools.mkdir()
            commands = {
                "just": '#!/bin/sh\nprintf "%s\\n" "$*" >> "$WORKER_BUILD_CALLS"\n',
                "cargo": (
                    '#!/bin/sh\nprintf "%s\\n" "$*" >> "$WORKER_BUILD_CALLS"\n'
                    'mkdir -p "$WORKER_BUILD_ROOT/target/debug"\n'
                    'touch "$WORKER_BUILD_ROOT/target/debug/hyperlight-sandbox-worker"\n'
                ),
            }
            for name, script in commands.items():
                executable = tools / name
                executable.write_text(script, encoding="utf-8")
                executable.chmod(0o755)
            calls = root / "calls"
            subprocess.run(
                [
                    just,
                    "--justfile",
                    str(repository / "src/sandbox_worker/Justfile"),
                    "--set",
                    "repo-root",
                    str(root),
                    "build",
                ],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "WORKER_BUILD_ROOT": str(root),
                    "WORKER_BUILD_CALLS": str(calls),
                },
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertTrue(
                (root / "target/debug/hyperlight-sandbox-worker").is_file(),
                "the Linux build recipe did not invoke Cargo",
            )
            self.assertIn("wasm guest-compile-wit", calls.read_text(encoding="utf-8"))
            self.assertIn(
                "build -p hyperlight-sandbox-worker --profile=dev",
                calls.read_text(encoding="utf-8"),
            )
