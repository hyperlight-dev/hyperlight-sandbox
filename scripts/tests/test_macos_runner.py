import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import tomllib


@unittest.skipIf(os.name == "nt", "The macOS Cargo runner uses Bash")
class MacosRunnerTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.repo = Path(__file__).resolve().parents[2]
        self.sign_log = self.root / "sign.log"
        self.executed = self.root / "executed"
        self.binary = self.root / "guest with spaces"
        self.binary.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\ntouch "$EXECUTED"\n')
        self.binary.chmod(0o755)
        codesign = self.root / "codesign"
        codesign.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$SIGN_LOG"\nexit "${SIGN_EXIT:-0}"\n')
        codesign.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.root}{os.pathsep}{os.environ['PATH']}",
            "SIGN_LOG": str(self.sign_log),
            "EXECUTED": str(self.executed),
        }

    def run_guest(self):
        config = tomllib.loads((self.repo / ".cargo" / "config.toml").read_text())
        runner = self.repo / config["target"]['cfg(target_os = "macos")']["runner"]
        return subprocess.run(
            [str(runner), str(self.binary), "argument with spaces", "--flag"],
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
        )

    def test_signs_with_hypervisor_entitlement_and_preserves_arguments(self):
        result = self.run_guest()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["argument with spaces", "--flag"])
        self.assertTrue(self.executed.exists())
        arguments = self.sign_log.read_text().splitlines()
        self.assertEqual(arguments[-1], str(self.binary))
        entitlement = self.repo / arguments[arguments.index("--entitlements") + 1]
        self.assertIn("<key>com.apple.security.hypervisor</key>", entitlement.read_text())
        self.assertIn("<true/>", entitlement.read_text())

    def test_does_not_run_unsigned_binary_after_signing_failure(self):
        self.env["SIGN_EXIT"] = "23"
        result = self.run_guest()
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertFalse(self.executed.exists())


if __name__ == "__main__":
    unittest.main()
