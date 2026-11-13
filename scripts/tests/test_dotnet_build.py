"""Check that .NET builds do not reuse stale native or guest assets."""

import copy
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path


@unittest.skipUnless(sys.platform == "linux", "Isolated Unix build regressions")
class DotnetBuildTests(unittest.TestCase):
    def test_build_guests_refreshes_staged_assets(self):
        repository = Path(__file__).resolve().parents[2]
        just = shutil.which("just")
        self.assertIsNotNone(just)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "bin"
            tools.mkdir()
            builder = tools / "just"
            builder.write_text(
                '#!/bin/sh\ncase "$*" in\n'
                "  *js-guest-build*) output=javascript/js-sandbox.aot ;;\n"
                "  *guest-build*) output=python/python-sandbox.aot ;;\n"
                "  *) exit 1 ;;\nesac\n"
                'printf new > "$DOTNET_BUILD_ROOT/src/wasm_sandbox/guests/$output"\n',
                encoding="utf-8",
            )
            builder.chmod(0o755)
            for name in ("python", "javascript"):
                (root / "src/wasm_sandbox/guests" / name).mkdir(parents=True)
            staged = root / "staged"
            staged.mkdir()
            for name in ("python-sandbox.aot", "js-sandbox.aot"):
                (staged / name).write_text("old", encoding="utf-8")
            subprocess.run(
                [
                    just,
                    "--justfile",
                    str(repository / "src/sdk/dotnet/Justfile"),
                    "--set",
                    "repo-root",
                    str(root),
                    "--set",
                    "guest-assets",
                    str(staged),
                    "build-guests",
                ],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "DOTNET_BUILD_ROOT": str(root),
                },
                check=True,
                capture_output=True,
                text=True,
            )
            for name in ("python-sandbox.aot", "js-sandbox.aot"):
                self.assertEqual((staged / name).read_text(encoding="utf-8"), "new")

    def test_managed_build_lets_cargo_check_workspace_changes(self):
        repository = Path(__file__).resolve().parents[2]
        original = ET.parse(repository / "src/sdk/dotnet/core/PInvoke/HyperlightSandbox.PInvoke.csproj").getroot()
        target = original.find("./Target[@Name='BuildRust']")
        self.assertIsNotNone(target)
        dotnet = shutil.which("dotnet")
        self.assertIsNotNone(dotnet)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tools = root / "bin"
            tools.mkdir()
            cargo = tools / "cargo"
            calls = root / "cargo-calls"
            cargo.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$DOTNET_BUILD_CALLS"\n',
                encoding="utf-8",
            )
            cargo.chmod(0o755)
            tracked = root / "ffi.rs"
            output = root / "target/debug/native.so"
            dependency = root / "core.rs"
            output.parent.mkdir(parents=True)
            for path, timestamp in ((tracked, 100), (output, 200), (dependency, 300)):
                path.write_text("changed", encoding="utf-8")
                os.utime(path, (timestamp, timestamp))
            project = ET.Element("Project")
            properties = ET.SubElement(project, "PropertyGroup")
            for name, value in {
                "WorkspaceRoot": str(root),
                "FfiCrateDir": str(root),
                "RustProfile": "debug",
                "NativeLibraryFileName": "native.so",
                "Configuration": "Debug",
            }.items():
                ET.SubElement(properties, name).text = value
            items = ET.SubElement(project, "ItemGroup")
            ET.SubElement(items, "RustFiles", Include=str(tracked))
            project.append(copy.deepcopy(target))
            project_path = root / "build.proj"
            ET.ElementTree(project).write(project_path, encoding="utf-8")
            subprocess.run(
                [dotnet, "msbuild", str(project_path), "-target:BuildRust", "-nologo"],
                cwd=root,
                env={
                    **os.environ,
                    "PATH": str(tools) + os.pathsep + os.environ["PATH"],
                    "DOTNET_BUILD_CALLS": str(calls),
                },
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertTrue(calls.is_file(), "MSBuild skipped Cargo's dependency checks")
