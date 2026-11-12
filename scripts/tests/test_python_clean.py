"""Check that Python cleanup removes staged native helpers."""

import subprocess
import tempfile
import unittest
from pathlib import Path


class PythonCleanTests(unittest.TestCase):
    def test_clean_removes_staged_workers_without_removing_package_files(self):
        repository = Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            packages = [
                root / "src/sdk/python/wasm_backend/hyperlight_sandbox_backend_wasm",
                root / "src/sdk/python/hyperlight_js_backend/hyperlight_sandbox_backend_hyperlight_js",
            ]
            for package in packages:
                package.mkdir(parents=True)
                (package / "hyperlight-sandbox-worker").write_text("staged helper", encoding="utf-8")
                (package / "__init__.py").touch()
            subprocess.run(
                [
                    "just",
                    "--justfile",
                    str(repository / "src/sdk/python/Justfile"),
                    "--set",
                    "repo-root",
                    str(root),
                    "clean",
                ],
                cwd=repository,
                check=True,
                capture_output=True,
                text=True,
            )
            for package in packages:
                with self.subTest(package=package.name):
                    self.assertFalse((package / "hyperlight-sandbox-worker").exists())
                    self.assertTrue((package / "__init__.py").exists())


if __name__ == "__main__":
    unittest.main()
