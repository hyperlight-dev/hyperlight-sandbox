import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from guest_build import cached_build


class GuestBuildCacheTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = self.root / "guest.py"
        self.wit = self.root / "world.wit"
        self.output = self.root / "guest.aot"
        self.source.write_text("source")
        self.wit.write_text("interface")
        self.calls = 0

    def build(self):
        self.calls += 1
        self.output.write_text(f"built-{self.calls}")

    def run_build(self, context="compiler-1"):
        cached_build(self.root, "guest", [self.source, self.wit], [self.output], context, self.build)

    def test_unchanged_inputs_reuse_output_across_invocations(self):
        self.run_build()
        self.run_build()
        self.assertEqual(self.calls, 1)

    def test_source_wit_and_compiler_changes_each_rebuild(self):
        self.run_build()
        self.source.write_text("changed source")
        self.run_build()
        self.wit.write_text("changed interface")
        self.run_build()
        self.run_build(context="compiler-2")
        self.assertEqual(self.calls, 4)

    def test_missing_or_modified_output_is_not_reused(self):
        self.run_build()
        self.output.unlink()
        self.run_build()
        self.output.write_text("stale cached artifact")
        self.run_build()
        self.assertEqual(self.calls, 3)

    def test_failed_build_cannot_publish_reusable_state(self):
        self.run_build()
        self.wit.write_text("new interface")

        def fail():
            self.output.write_text("partial output")
            raise subprocess.CalledProcessError(1, ["compiler"])

        with self.assertRaises(subprocess.CalledProcessError):
            cached_build(self.root, "guest", [self.source, self.wit], [self.output], "compiler-1", fail)
        self.run_build()
        self.assertEqual(self.calls, 2)
        self.assertEqual(self.output.read_text(), "built-2")

    def test_success_without_required_output_is_an_error(self):
        with self.assertRaises(FileNotFoundError):
            cached_build(self.root, "guest", [self.source], [self.output], "", lambda: None)

    def test_missing_input_does_not_reuse_previous_build(self):
        self.run_build()
        self.wit.unlink()
        with self.assertRaises(FileNotFoundError):
            self.run_build()


if __name__ == "__main__":
    unittest.main()
