"""Exercise the macOS LLVM setup without changing Homebrew links."""

import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


@unittest.skipIf(sys.platform == "win32", "Exercises POSIX workflow setup")
class MacosLlvmTests(unittest.TestCase):
    def test_reports_an_unlinked_install_without_linking(self):
        self.check_setup(linked=False)

    def test_keeps_a_working_install_linked(self):
        self.check_setup(linked=True)

    def test_reports_compiler_failure(self):
        self.check_setup(linked=True, compiler_fails=True)

    def check_setup(self, *, linked, compiler_fails=False):
        workflows = Path(__file__).resolve().parents[2] / ".github/workflows"
        count = 0
        for workflow in workflows.glob("*.yml"):
            for block in re.findall(
                r"(?m)^        run: \|\n((?:          .*\n|\n)+)",
                workflow.read_text(encoding="utf-8"),
            ):
                if "brew install llvm" not in block or "llvm_bin=" not in block:
                    continue
                count += 1
                with self.subTest(workflow=workflow.name, setup=count):
                    self.run_setup(textwrap.dedent(block), linked, compiler_fails)
        self.assertGreater(count, 0)

    def run_setup(self, script, linked, compiler_fails):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "bin"
            tools.mkdir()
            keg = root / "Cellar/llvm/23.1.5_1"
            (keg / "bin").mkdir(parents=True)
            (keg / "lib").mkdir()
            clang = keg / "bin/clang"
            clang.write_text(
                "#!/bin/sh\n" + ("exit 7\n" if compiler_fails else "echo 'clang version 23'\n"),
                encoding="utf-8",
            )
            clang.chmod(0o755)
            prefix = root / "opt/llvm"
            prefix.parent.mkdir()
            if linked:
                prefix.symlink_to(keg, target_is_directory=True)
            brew = tools / "brew"
            brew.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$LLVM_CALLS"\n'
                'case "$*" in\n'
                '  "install llvm") exit 0 ;;\n'
                '  "--prefix llvm") printf "%s\\n" "$LLVM_PREFIX" ;;\n'
                '  "link --force llvm") exit 7 ;;\n'
                "  *) exit 8 ;;\nesac\n",
                encoding="utf-8",
            )
            brew.chmod(0o755)
            calls = root / "calls"
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", script],
                env={
                    **os.environ,
                    "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "LLVM_PREFIX": str(prefix),
                    "LLVM_CALLS": str(calls),
                    "GITHUB_PATH": str(root / "github-path"),
                    "GITHUB_ENV": str(root / "github-env"),
                },
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertNotIn("link --force llvm", calls.read_text(encoding="utf-8"))
            self.assertEqual(prefix.is_symlink(), linked)
            if not linked or compiler_fails:
                self.assertEqual(result.returncode, 127 if not linked else 7, result.stderr)
                self.assertFalse((root / "github-path").exists())
                self.assertFalse((root / "github-env").exists())
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    (root / "github-path").read_text(encoding="utf-8").strip(),
                    str(prefix / "bin"),
                )
                self.assertEqual(
                    (root / "github-env").read_text(encoding="utf-8").strip(),
                    f"LIBCLANG_PATH={prefix / 'lib'}",
                )
