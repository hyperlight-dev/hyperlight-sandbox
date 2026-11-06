"""Keep shared guest artifacts out of Rust cache restore cleanup."""

import re
import unittest
from pathlib import Path


class GuestArtifactWorkflowTests(unittest.TestCase):
    def test_guest_download_follows_rust_cache_restore(self):
        workflow = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
        jobs = re.split(r"(?m)^  ([\w-]+):\s*$", workflow.read_text(encoding="utf-8").split("jobs:\n", 1)[1])
        consumers = 0
        for name, job in zip(jobs[1::2], jobs[2::2], strict=True):
            if 'HYPERLIGHT_GUEST_ARTIFACT: "1"' not in job:
                continue
            consumers += 1
            with self.subTest(job=name):
                restore = job.index("- uses: actions-rust-lang/setup-rust-toolchain@")
                download = job.index("- uses: actions/download-artifact@")
                self.assertGreater(download, restore, "Rust cache restore can delete the downloaded guest manifest")
        self.assertGreater(consumers, 0)


if __name__ == "__main__":
    unittest.main()
