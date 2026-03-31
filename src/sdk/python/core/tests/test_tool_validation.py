"""Tests for tool argument validation in the hyperlight-sandbox Python SDK.

Exercises schema inference, type validation, payload limits, and edge cases
for tool registration and dispatch. Runs against a real Wasm sandbox.

Usage:
    uv run python -m unittest src/sdk/python/core/tests/test_tool_validation.py -v
"""

import unittest
from typing import Annotated

from hyperlight_sandbox import Sandbox


def _make_sandbox(**tools) -> Sandbox:
    """Create a Wasm sandbox with the given tools registered."""
    s = Sandbox(backend="wasm")
    for name, cb in tools.items():
        s.register_tool(name, cb)
    return s


class TestTypedDefaultsInference(unittest.TestCase):
    """Schema inference from default values — the most common pattern."""

    def test_number_defaults_reject_string_arg(self):
        """lambda a=0, b=0 should reject string arguments."""
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        result = s.run("call_tool('add', a='three', b=4)")
        self.assertNotEqual(result.exit_code, 0, f"Should have failed but stdout={result.stdout!r}")

    def test_number_defaults_accept_number_arg(self):
        """lambda a=0, b=0 should accept numeric arguments."""
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        result = s.run("print(call_tool('add', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("7", result.stdout)

    def test_string_default_rejects_number_arg(self):
        """lambda name='world' should reject numeric arguments."""
        s = _make_sandbox(greet=lambda name="world": f"Hello, {name}!")
        result = s.run("call_tool('greet', name=42)")
        self.assertNotEqual(result.exit_code, 0, f"Should have failed but stdout={result.stdout!r}")

    def test_string_default_accepts_string_arg(self):
        """lambda name='world' should accept string arguments."""
        s = _make_sandbox(greet=lambda name="world": f"Hello, {name}!")
        result = s.run("print(call_tool('greet', name='Alice'))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("Hello, Alice!", result.stdout)

    def test_bool_default_rejects_string_arg(self):
        """lambda flag=True should reject string arguments."""
        s = _make_sandbox(check=lambda flag=True: flag)
        result = s.run("call_tool('check', flag='yes')")
        self.assertNotEqual(result.exit_code, 0, f"Should have failed but stdout={result.stdout!r}")

    def test_bool_default_accepts_bool_arg(self):
        """lambda flag=True should accept boolean arguments."""
        s = _make_sandbox(check=lambda flag=True: flag)
        result = s.run("print(call_tool('check', flag=False))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")


class TestAnnotationInference(unittest.TestCase):
    """Schema inference from function type annotations."""

    def test_annotated_float_rejects_string(self):
        def add(a: float, b: float) -> float:
            return a + b

        s = _make_sandbox(add=add)
        result = s.run("call_tool('add', a='bad', b=4)")
        self.assertNotEqual(result.exit_code, 0)

    def test_annotated_float_accepts_number(self):
        def add(a: float, b: float) -> float:
            return a + b

        s = _make_sandbox(add=add)
        result = s.run("print(call_tool('add', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("7", result.stdout)

    def test_annotated_str_rejects_number(self):
        def greet(name: str) -> str:
            return f"Hello, {name}!"

        s = _make_sandbox(greet=greet)
        result = s.run("call_tool('greet', name=42)")
        self.assertNotEqual(result.exit_code, 0)

    def test_typing_annotated_unwraps_base_type(self):
        """Annotated[str, ...] should unwrap to str and reject numbers."""
        try:
            from pydantic import Field
        except ImportError:
            self.skipTest("pydantic not installed")

        def fetch(table: Annotated[str, Field(description="table name")]) -> list:
            return []

        s = _make_sandbox(fetch=fetch)
        result = s.run("call_tool('fetch', table=123)")
        self.assertNotEqual(result.exit_code, 0, f"Should have failed but stdout={result.stdout!r}")


class TestAsyncTools(unittest.TestCase):
    """Async tool handlers should work and get schema inference."""

    def test_async_with_defaults_validates_types(self):
        import asyncio

        async def multiply(a=0, b=0):
            await asyncio.sleep(0)
            return a * b

        s = _make_sandbox(multiply=multiply)
        # Wrong type
        result = s.run("call_tool('multiply', a='bad', b=4)")
        self.assertNotEqual(result.exit_code, 0)
        # Correct type
        result = s.run("print(call_tool('multiply', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("12", result.stdout)


class TestEdgeCases(unittest.TestCase):
    """Edge cases for tool registration and dispatch."""

    def test_positional_only_builtin_errors_at_runtime(self):
        """Built-in `len` has POSITIONAL_ONLY params — kwargs dispatch fails at runtime."""
        s = _make_sandbox(length=len)
        result = s.run("""
try:
    call_tool('length', obj=[1,2,3])
    print('SHOULD NOT REACH')
except RuntimeError as e:
    print(f'caught: {e}')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("caught:", result.stdout)
        self.assertNotIn("SHOULD NOT REACH", result.stdout)

    def test_no_args_tool_works(self):
        """A tool with no parameters should work fine."""
        s = _make_sandbox(ping=lambda: "pong")
        result = s.run("print(call_tool('ping'))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("pong", result.stdout)


class TestSchemaErrorsAreCatchableByGuest(unittest.TestCase):
    """Schema validation errors should be catchable RuntimeErrors in the guest.

    The host must never crash. The guest gets an error it can handle gracefully.
    """

    def test_type_error_catchable_and_guest_continues(self):
        """Guest catches type validation error and continues running."""
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        result = s.run("""
try:
    call_tool('add', a='not a number', b=4)
    print('SHOULD NOT REACH')
except RuntimeError as e:
    print(f'caught: {e}')
print('guest still alive')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("caught:", result.stdout)
        self.assertIn("must be number", result.stdout)
        self.assertIn("guest still alive", result.stdout)
        self.assertNotIn("SHOULD NOT REACH", result.stdout)

    def test_missing_arg_error_catchable(self):
        """Missing required arg error is catchable."""

        def add(a, b):
            return a + b

        s = _make_sandbox(add=add)
        result = s.run("""
try:
    call_tool('add', a=3)
    print('SHOULD NOT REACH')
except RuntimeError as e:
    print(f'caught: {e}')
print('still running')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("caught:", result.stdout)
        self.assertIn("missing required", result.stdout)
        self.assertIn("still running", result.stdout)

    def test_uncaught_error_exits_with_code_1_not_host_crash(self):
        """Uncaught schema error exits guest with code 1 — host is still fine."""
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        # No try/except — uncaught RuntimeError
        result = s.run("call_tool('add', a=[1,2,3], b=4)")
        self.assertEqual(result.exit_code, 1)
        self.assertIn("must be number, got array", result.stderr)
        # Host is fine — we can run again
        result = s.run("print('host survived')")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("host survived", result.stdout)

    def test_uncaught_untyped_error_exits_with_code_1_not_host_crash(self):
        """Untyped tool: wrong types pass schema but fail in handler — host survives."""
        s = _make_sandbox(add=lambda a, b: a + b)
        # No try/except — [1,2,3] + 4 causes TypeError in the handler
        result = s.run("call_tool('add', a=[1,2,3], b=4)")
        self.assertEqual(result.exit_code, 1)
        # Error comes from Python handler, not schema validation
        self.assertTrue(
            "TypeError" in result.stderr
            or "unsupported operand" in result.stderr
            or "can only concatenate" in result.stderr,
            f"Expected handler TypeError, got: {result.stderr[:200]}",
        )
        # Host is fine — we can run again
        result = s.run("print('host survived')")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("host survived", result.stdout)

    def test_multiple_errors_in_sequence_host_stays_healthy(self):
        """Multiple bad calls in sequence — host never degrades."""
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        for i in range(5):
            result = s.run(f"""
try:
    call_tool('add', a='bad{i}', b=4)
except RuntimeError:
    pass
print('iteration {i} ok')
""")
            self.assertEqual(result.exit_code, 0, f"Failed at iteration {i}: {result.stderr}")
            self.assertIn(f"iteration {i} ok", result.stdout)


class TestMissingRequiredArgs(unittest.TestCase):
    """Required arguments (no default) must be present."""

    def test_missing_required_arg_errors(self):
        def add(a: float, b: float) -> float:
            return a + b

        s = _make_sandbox(add=add)
        result = s.run("call_tool('add', a=3)")
        self.assertNotEqual(result.exit_code, 0)

    def test_all_required_args_present_succeeds(self):
        def add(a: float, b: float) -> float:
            return a + b

        s = _make_sandbox(add=add)
        result = s.run("print(call_tool('add', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")


class TestUntypedFunctions(unittest.TestCase):
    """Functions with no type annotations and no typed defaults.

    These have required_untyped args — presence is checked but type is not.
    This is the weakest validation level; the handler is responsible for
    type safety.
    """

    def test_bare_lambda_accepts_numbers(self):
        """lambda a, b: a + b — no defaults, no annotations."""
        s = _make_sandbox(add=lambda a, b: a + b)
        result = s.run("print(call_tool('add', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("7", result.stdout)

    def test_bare_lambda_accepts_strings_no_type_check(self):
        """Without types, strings pass validation (only presence is checked)."""
        s = _make_sandbox(add=lambda a, b: a + b)
        result = s.run("print(call_tool('add', a='hello', b=' world'))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("hello world", result.stdout)

    def test_bare_lambda_rejects_missing_arg(self):
        """Presence is still enforced even without types."""
        s = _make_sandbox(add=lambda a, b: a + b)
        result = s.run("call_tool('add', a=3)")
        self.assertNotEqual(result.exit_code, 0)

    def test_bare_def_no_annotations_accepts_any_type(self):
        """def without annotations — any type passes, only presence checked."""

        def concat(x, y):
            return f"{x}-{y}"

        s = _make_sandbox(concat=concat)
        # Numbers pass
        result = s.run("print(call_tool('concat', x=1, y=2))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("1-2", result.stdout)
        # Strings pass
        result = s.run("print(call_tool('concat', x='a', y='b'))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("a-b", result.stdout)
        # Mixed types pass
        result = s.run("print(call_tool('concat', x=42, y='hello'))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("42-hello", result.stdout)

    def test_bare_def_rejects_missing_required(self):
        """Even without annotations, missing args are caught."""

        def concat(x, y):
            return f"{x}-{y}"

        s = _make_sandbox(concat=concat)
        result = s.run("call_tool('concat', x=1)")
        self.assertNotEqual(result.exit_code, 0)

    def test_kwargs_wrapper_no_validation(self):
        """lambda **kw — no params to inspect, schema is empty."""

        def real_add(a, b):
            return a + b

        s = _make_sandbox(add=lambda **kw: real_add(**kw))
        # Correct types work
        result = s.run("print(call_tool('add', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("7", result.stdout)
        # Wrong types also pass schema (no validation) — fails in handler
        result = s.run("""
try:
    call_tool('add', a='x', b='y')
except RuntimeError:
    pass
print('no crash')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("no crash", result.stdout)

    def test_mixed_typed_and_untyped_params(self):
        """def with one annotated and one bare param."""

        def mixed(a: float, b):
            return a + float(b)

        s = _make_sandbox(mixed=mixed)
        # a is typed, b is untyped
        result = s.run("call_tool('mixed', a='bad', b=4)")
        self.assertNotEqual(result.exit_code, 0, "a should be validated as number")
        # a correct, b anything — passes schema
        result = s.run("print(call_tool('mixed', a=3, b=4))")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("7", result.stdout)


class TestTypeValidationMatrix(unittest.TestCase):
    """Deterministic type validation matrix.

    Sends each of 6 JSON types to tools expecting each of 5 ArgTypes.
    Correct type should pass. Wrong types should be rejected. Host never crashes.
    For randomized/adversarial testing, see fuzz_tool_dispatch.py (atheris).
    """

    def test_every_json_type_against_every_argtype(self):
        s = _make_sandbox(
            num_tool=lambda x=0: x,
            str_tool=lambda x="": x,
            bool_tool=lambda x=True: x,
            list_tool=lambda x=[]: x,
            dict_tool=lambda x={}: x,
        )
        test_values = [
            ("None", "null"),
            ("42", "number"),
            ("'hello'", "string"),
            ("True", "boolean"),
            ("[1, 2]", "array"),
            ("{'k': 'v'}", "object"),
        ]
        tools = [
            ("num_tool", "number"),
            ("str_tool", "string"),
            ("bool_tool", "boolean"),
            ("list_tool", "array"),
            ("dict_tool", "object"),
        ]
        for tool_name, expected_type in tools:
            for val_code, val_type in test_values:
                should_pass = val_type == expected_type
                code = f"""
try:
    call_tool('{tool_name}', x={val_code})
    print('passed')
except RuntimeError:
    print('rejected')
print('ok')
"""
                result = s.run(code)
                self.assertEqual(
                    result.exit_code, 0, f"Host crashed: {tool_name}(x={val_code}), stderr={result.stderr}"
                )
                if should_pass:
                    self.assertIn("passed", result.stdout, f"{tool_name}(x={val_code}) should pass (both {val_type})")
                else:
                    self.assertIn(
                        "rejected",
                        result.stdout,
                        f"{tool_name}(x={val_code}) should reject ({val_type} != {expected_type})",
                    )


class TestUnknownTool(unittest.TestCase):
    """Calling a tool that doesn't exist should return an error."""

    def test_unknown_tool_returns_error(self):
        s = _make_sandbox(add=lambda a=0, b=0: a + b)
        result = s.run("""
try:
    call_tool('nonexistent', x=1)
    print('SHOULD NOT REACH')
except RuntimeError as e:
    print(f'caught: {e}')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("caught:", result.stdout)
        self.assertNotIn("SHOULD NOT REACH", result.stdout)


class TestCodeInjectionSafety(unittest.TestCase):
    """Verify that code-like strings in tool arguments are treated as inert data.

    These tests pass payloads that would kill the host process if executed.
    If the test finishes at all, the payload was not executed.
    """

    def setUp(self):
        self.sandbox = _make_sandbox(echo=lambda data="": data)

    def test_sys_exit_in_args_does_not_kill_host(self):
        """If this payload were eval'd on the host, the test process would exit."""
        payload = "__import__('sys').exit(99)"
        result = self.sandbox.run(f"""print(repr(call_tool('echo', data="{payload}")))""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn(payload, result.stdout, "payload was not returned verbatim — it may have been executed")

    def test_os_abort_in_args_does_not_kill_host(self):
        """If this payload were eval'd on the host, the test process would SIGABRT."""
        payload = "__import__('os')._exit(1)"
        result = self.sandbox.run(f"""print(repr(call_tool('echo', data="{payload}")))""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn(payload, result.stdout, "payload was not returned verbatim — it may have been executed")

    def test_null_bytes_survive_round_trip(self):
        """Null bytes in strings must not truncate or corrupt data."""
        result = self.sandbox.run(r"""print(repr(call_tool('echo', data="before\x00after")))""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("before", result.stdout)
        self.assertIn("after", result.stdout)

    def test_deeply_nested_json_does_not_crash(self):
        """Deeply nested objects must not stack-overflow the parser."""
        # Build 50 levels of nesting via guest code
        result = self.sandbox.run("""
import json
obj = "leaf"
for _ in range(50):
    obj = {"d": obj}
result = call_tool('echo', data=json.dumps(obj))
# Verify it survived
print('nested_ok' if isinstance(result, str) else 'nested_fail')
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("nested_ok", result.stdout)

    def test_unicode_edge_cases_survive_round_trip(self):
        """RTL overrides, zero-width chars, and BOM must pass through intact."""
        result = self.sandbox.run(r"""
val = call_tool('echo', data="admin\u202etxt.exe")
print(repr(val))
""")
        self.assertEqual(result.exit_code, 0, f"stderr: {result.stderr}")
        self.assertIn("\\u202e", result.stdout.lower(), "RTL override char must survive round-trip")


if __name__ == "__main__":
    unittest.main()
