"""Check that the final CI job covers every job and propagates unsuccessful results."""

import json
import os
import re
import subprocess
import textwrap
import unittest
from pathlib import Path


class CiStatusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = (Path(__file__).resolve().parents[2] / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        sections = re.split(r"(?m)^  ([\w-]+):\s*$", workflow.split("\njobs:\n", 1)[1])
        cls.jobs = dict(zip(sections[1::2], sections[2::2], strict=True))
        cls.report = cls.jobs["report-ci-status"]

    def test_always_waits_for_every_other_job(self):
        needs = re.search(r"(?m)^    needs:\n((?:      - [\w-]+\n)+)", self.report)
        self.assertIsNotNone(needs)
        dependencies = re.findall(r"(?m)^      - ([\w-]+)$", needs.group(1))
        self.assertEqual(set(dependencies), self.jobs.keys() - {"report-ci-status"})
        self.assertRegex(self.report, r"(?m)^    if: always\(\)$")

    def test_reports_failed_and_cancelled_jobs_but_allows_skipped_jobs(self):
        block = re.search(r"(?m)^          script: \|\n((?:            .*\n)+)", self.report)
        self.assertIsNotNone(block)
        script = textwrap.dedent(block.group(1))
        for results, expected in (
            (["success", "success"], 0),
            (["success", "skipped"], 0),
            (["failure", "success"], 1),
            (["success", "failure"], 1),
            (["cancelled", "success"], 1),
            (["success", "cancelled"], 1),
            (["failure", "skipped"], 1),
            (["cancelled", "skipped"], 1),
        ):
            with self.subTest(results=results):
                result = subprocess.run(
                    [
                        "node",
                        "-e",
                        "const core = {setFailed(message) {console.error(message); process.exitCode = 1;}};\n" + script,
                    ],
                    env={**os.environ, "JOB_RESULTS": json.dumps(results)},
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
