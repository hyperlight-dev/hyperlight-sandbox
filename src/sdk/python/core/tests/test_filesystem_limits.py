"""Tests for stable filesystem quota validation and forwarding."""

import unittest
from unittest.mock import patch

from hyperlight_sandbox import CodeExecutionTool, Sandbox, SandboxEnvironment


class _FakeNativeSandbox:
    calls: list[dict] = []

    def __init__(self, **kwargs):
        self.calls.append(kwargs)

    def register_tool(self, *args):
        pass


class TestFilesystemLimitForwarding(unittest.TestCase):
    def setUp(self):
        _FakeNativeSandbox.calls.clear()

    def test_omitted_limits_leave_backend_defaults_unchanged(self):
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("hyperlight-js", _FakeNativeSandbox),
        ):
            Sandbox(backend="hyperlight-js")

        kwargs = _FakeNativeSandbox.calls[-1]
        self.assertNotIn("filesystem_limits", kwargs)
        self.assertNotIn("max_file_size", kwargs)
        self.assertNotIn("max_total_size", kwargs)
        self.assertNotIn("max_file_count", kwargs)

    def test_custom_limits_and_zero_are_forwarded(self):
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("hyperlight-js", _FakeNativeSandbox),
        ):
            Sandbox(
                backend="hyperlight-js",
                max_file_size="8Mi",
                max_total_size="20Mi",
                max_file_count=0,
            )

        self.assertEqual(_FakeNativeSandbox.calls[-1]["max_file_size"], "8Mi")
        self.assertEqual(_FakeNativeSandbox.calls[-1]["max_total_size"], "20Mi")
        self.assertEqual(_FakeNativeSandbox.calls[-1]["max_file_count"], 0)

    def test_unlimited_is_forwarded(self):
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("hyperlight-js", _FakeNativeSandbox),
        ):
            Sandbox(backend="hyperlight-js", filesystem_limits="unlimited")

        self.assertEqual(_FakeNativeSandbox.calls[-1]["filesystem_limits"], "unlimited")

    def test_wasm_backend_forwards_limits(self):
        with (
            patch(
                "hyperlight_sandbox._load_backend",
                return_value=("wasm", _FakeNativeSandbox),
            ),
            patch("hyperlight_sandbox.resolve_module_path", return_value="/guest.wasm"),
        ):
            Sandbox(backend="wasm", max_total_size="4Mi")

        kwargs = _FakeNativeSandbox.calls[-1]
        self.assertEqual(kwargs["module_path"], "/guest.wasm")
        self.assertEqual(kwargs["max_total_size"], "4Mi")

    def test_environment_forwards_limits(self):
        environment = SandboxEnvironment(max_file_size="1Ki", max_file_count=2)
        tool = CodeExecutionTool(environment=environment)
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("hyperlight-js", _FakeNativeSandbox),
        ):
            tool._get_sandbox()

        kwargs = _FakeNativeSandbox.calls[-1]
        self.assertEqual(kwargs["max_file_size"], "1Ki")
        self.assertEqual(kwargs["max_file_count"], 2)


class TestFilesystemLimitValidation(unittest.TestCase):
    def test_unlimited_rejects_numeric_overrides(self):
        with self.assertRaisesRegex(ValueError, "cannot be combined"):
            Sandbox(filesystem_limits="unlimited", max_file_count=0)

    def test_unknown_mode_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "must be 'unlimited'"):
            Sandbox(filesystem_limits="default")

    def test_negative_file_count_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "non-negative"):
            Sandbox(max_file_count=-1)

    def test_file_count_rejects_bool(self):
        with self.assertRaisesRegex(TypeError, "must be an integer"):
            Sandbox(max_file_count=True)

    def test_size_override_must_be_a_string(self):
        with self.assertRaisesRegex(TypeError, "must be a size string"):
            Sandbox(max_file_size=1024)  # type: ignore[arg-type]

    def test_new_kwargs_are_not_silently_dropped_for_old_backends(self):
        class _OldBackend:
            def __init__(self, *, heap_size, stack_size):
                pass

        with (
            patch(
                "hyperlight_sandbox._load_backend",
                return_value=("hyperlight-js", _OldBackend),
            ),
            self.assertRaisesRegex(TypeError, "max_file_count"),
        ):
            Sandbox(backend="hyperlight-js", max_file_count=1)


if __name__ == "__main__":
    unittest.main()
