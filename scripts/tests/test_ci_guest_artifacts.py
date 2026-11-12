"""Keep generated build artifacts out of Rust cache cleanup."""

import re
import unittest
from pathlib import Path


class GuestArtifactWorkflowTests(unittest.TestCase):
    def test_fork_pull_requests_include_macos_jobs(self):
        workflow = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
        jobs = re.split(r"(?m)^  ([\w-]+):\s*$", workflow.read_text(encoding="utf-8").split("jobs:\n", 1)[1])
        macos_jobs = 0
        for name, job in zip(jobs[1::2], jobs[2::2], strict=True):
            if "macos-hvf" not in job and "-macos.yml" not in job:
                continue
            macos_jobs += 1
            with self.subTest(job=name):
                self.assertNotIn("github.event.pull_request.head.repo", job)
                if "macos-hvf" in job:
                    matrix = job.split("matrix:", 1)[1].split("\n    steps:", 1)[0]
                    self.assertIn("os: [ubuntu-latest, windows-latest, macos-hvf]", matrix)
        self.assertGreater(macos_jobs, 0)

    def test_macos_environment_precedes_rust_setup(self):
        workflow = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
        jobs = re.split(r"(?m)^  ([\w-]+):\s*$", workflow.read_text(encoding="utf-8").split("jobs:\n", 1)[1])
        macos_jobs = 0
        for name, job in zip(jobs[1::2], jobs[2::2], strict=True):
            if "macos-hvf" not in job:
                continue
            macos_jobs += 1
            with self.subTest(job=name):
                setup = job.index("name: Set up macOS environment")
                rust = job.index("- uses: actions-rust-lang/setup-rust-toolchain@")
                self.assertLess(setup, rust, "Rust setup calls brew before installing the toolchain")
                step = job[setup:].split("\n      - ", 1)[0]
                self.assertIn("if: runner.os == 'macOS'", step)
                self.assertIn('prefix="$HOME/.homebrew"', step)
                self.assertIn('"$prefix/bin"', step)
                self.assertIn('"$prefix/sbin"', step)
                self.assertIn("$HOME/.cargo/bin", step)
                self.assertIn('"$GITHUB_PATH"', step)
        self.assertGreater(macos_jobs, 0)

    def test_windows_surrogate_cache_follows_rust_cache_restore(self):
        workflow = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
        rust_job = workflow.read_text(encoding="utf-8").split("\n  rust:\n", 1)[1].split("\n  wasm-sandbox:", 1)[0]
        self.assertIn("name: Cache Windows surrogate", rust_job)
        restore = rust_job.index("- uses: actions-rust-lang/setup-rust-toolchain@")
        surrogate = rust_job.index("name: Cache Windows surrogate")
        lint = rust_job.index("name: Lint")
        self.assertLess(restore, surrogate, "Rust cache cleanup removes the nested surrogate build directory")
        self.assertLess(surrogate, lint, "The surrogate must be restored before compiling hyperlight-host")
        surrogate_step = rust_job[surrogate:].split("\n      - ", 1)[0]
        self.assertIn("if: runner.os == 'Windows'", surrogate_step)
        self.assertIn("uses: actions/cache@", surrogate_step)
        self.assertIn("target/debug/build/hls/x86_64-pc-windows-msvc/debug/hyperlight_surrogate.exe", surrogate_step)
        self.assertIn("steps.rust-toolchain.outputs.cachekey", surrogate_step)
        self.assertIn("hashFiles('Cargo.lock', 'rust-toolchain.toml')", surrogate_step)
        repair = rust_job.index("run: just clean-rust-host-cache")
        self.assertLess(surrogate, repair)
        self.assertLess(repair, lint)
        repair_step = rust_job[surrogate:].split("\n      - ", 2)[1]
        self.assertIn("runner.os == 'Windows' && steps.windows-surrogate.outputs.cache-hit != 'true'", repair_step)

    def test_guest_download_follows_rust_cache_restore(self):
        workflow = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
        jobs = re.split(r"(?m)^  ([\w-]+):\s*$", workflow.read_text(encoding="utf-8").split("jobs:\n", 1)[1])
        consumers = 0
        for name, job in zip(jobs[1::2], jobs[2::2], strict=True):
            if "HYPERLIGHT_GUEST_ARTIFACT:" not in job:
                continue
            consumers += 1
            with self.subTest(job=name):
                restore = job.index("- uses: actions-rust-lang/setup-rust-toolchain@")
                download = job.index("- uses: actions/download-artifact@")
                self.assertGreater(download, restore, "Rust cache restore can delete the downloaded guest manifest")
        self.assertGreater(consumers, 0)


if __name__ == "__main__":
    unittest.main()
