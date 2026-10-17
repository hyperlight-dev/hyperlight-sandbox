"""Tests for how Sandbox maps its options onto the unikraft backend."""

import unittest
from unittest.mock import patch

from hyperlight_sandbox import CodeExecutionTool, Sandbox, SandboxEnvironment, _normalize_backend


class _FakeNativeSandbox:
    calls: list[dict] = []

    def __init__(self, **kwargs):
        self.calls.append(kwargs)


class TestUnikraftOptions(unittest.TestCase):
    def setUp(self):
        _FakeNativeSandbox.calls.clear()

    def _build(self, **kwargs):
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("unikraft", _FakeNativeSandbox),
        ):
            Sandbox(backend="unikraft", **kwargs)
        return _FakeNativeSandbox.calls[-1]

    def test_defaults_leave_the_image_to_the_backend(self):
        kwargs = self._build()
        # The Wasm guest default and Wasm memory sizes do not apply.
        self.assertNotIn("runtime", kwargs)
        self.assertNotIn("module_path", kwargs)
        self.assertNotIn("heap_size", kwargs)
        self.assertNotIn("stack_size", kwargs)

    def test_unikraft_options_are_forwarded(self):
        kwargs = self._build(module="python", scratch_size="2Gi", native_network=True)
        self.assertEqual(kwargs["runtime"], "python")
        self.assertEqual(kwargs["scratch_size"], "2Gi")
        self.assertTrue(kwargs["native_network"])

    def test_module_path_names_the_rootfs(self):
        kwargs = self._build(module_path="/images/agent.cpio")
        self.assertEqual(kwargs["module_path"], "/images/agent.cpio")
        self.assertNotIn("runtime", kwargs)

    def test_options_are_checked_before_any_backend_loads(self):
        # No backend need be installed to be told an option is wrong.
        with patch("hyperlight_sandbox._load_backend", side_effect=ImportError) as load:
            with self.assertRaises(ValueError):
                Sandbox(backend="hyperlight-js", scratch_size="2Gi")
            with self.assertRaises(ValueError):
                Sandbox(backend="wasm", native_network=True)
            with self.assertRaises(ValueError):
                Sandbox(backend="unikraft", heap_size="64Mi")
            with self.assertRaises(ValueError):
                Sandbox(backend="unikraft", stack_size="1Mi")
            with self.assertRaises(ValueError):
                Sandbox(backend="unikraft", module="python", module_path="/images/agent.cpio")
        load.assert_not_called()

    def test_code_execution_tool_builds_a_unikraft_sandbox(self):
        tool = CodeExecutionTool(
            environment=SandboxEnvironment(backend="unikraft", module="python", scratch_size="512Mi")
        )
        with patch(
            "hyperlight_sandbox._load_backend",
            return_value=("unikraft", _FakeNativeSandbox),
        ):
            tool._get_sandbox()
        kwargs = _FakeNativeSandbox.calls[-1]
        self.assertEqual(kwargs["runtime"], "python")
        self.assertEqual(kwargs["scratch_size"], "512Mi")
        self.assertNotIn("heap_size", kwargs)

    def test_backend_aliases(self):
        for alias in ("unikraft", "hyperlight-unikraft", "hluk"):
            self.assertEqual(_normalize_backend(alias), "unikraft")


if __name__ == "__main__":
    unittest.main()
