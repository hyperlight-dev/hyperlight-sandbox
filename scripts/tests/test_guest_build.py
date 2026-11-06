import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from guest_build import GuestBuilder, cached_build, fingerprint, output_hashes, write_state


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

    def test_artifact_fingerprint_is_portable_across_checkout_line_endings(self):
        self.source.write_bytes(b"first line\nsecond line\n")
        original = fingerprint(self.root, [self.source], "guest-bundle")
        self.source.write_bytes(b"first line\r\nsecond line\r\n")
        self.assertEqual(fingerprint(self.root, [self.source], "guest-bundle"), original)

    def test_python_support_and_type_stub_changes_rebuild(self):
        builder = GuestBuilder(self.root)
        python = builder.wasm / "guests" / "python"
        support = [
            python / "componentize_py_async_support" / "futures.py",
            python / "componentize_py_runtime.pyi",
            python / "componentize_py_async_support" / "types.pyi",
        ]
        for path in [*builder.inputs(), *support]:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("original")
        cached_build(self.root, "guest", builder.inputs(), [self.output], "compiler-1", self.build)
        for path in support:
            with self.subTest(path=path.relative_to(self.root)):
                calls = self.calls
                path.write_text("changed")
                cached_build(self.root, "guest", builder.inputs(), [self.output], "compiler-1", self.build)
                self.assertEqual(self.calls, calls + 1)

    def test_shared_artifact_rejects_changed_wit_or_output(self):
        builder = GuestBuilder(self.root)
        builder.inputs = lambda: [self.source, self.wit]
        builder.outputs = lambda: [self.output]
        self.build()
        stamp = self.root / "target" / "guest-build" / "bundle.json"
        write_state(
            stamp,
            {
                "inputs": fingerprint(self.root, builder.inputs(), "guest-bundle"),
                "outputs": output_hashes(self.root, builder.outputs()),
            },
        )
        builder.verify_bundle()
        self.output.write_text("wrong artifact")
        with self.assertRaises(RuntimeError):
            builder.verify_bundle()
        self.output.write_text("built-1")
        self.wit.write_text("different interface")
        with self.assertRaises(RuntimeError):
            builder.verify_bundle()

    def test_shared_artifact_requires_a_manifest(self):
        builder = GuestBuilder(self.root)
        with self.assertRaises(RuntimeError):
            builder.verify_bundle()


if __name__ == "__main__":
    unittest.main()
