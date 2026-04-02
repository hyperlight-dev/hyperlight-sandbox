"""Hyperlight Sandbox — JavaScript basics example (HyperlightJS backend).

Exercises: basic execution, tool dispatch, snapshot/restore, complex
computation, and nested tool calls.

File I/O tests live in javascript_filesystem_demo.py.
Network tests live in javascript_network_demo.py.
"""

import time

from hyperlight_sandbox import Sandbox


def timed_run(sandbox, code, label="run"):
    start = time.perf_counter()
    result = sandbox.run(code)
    elapsed_ms = (time.perf_counter() - start) * 1000
    print(f"[timer]  {label}: {elapsed_ms:.1f}ms")
    return result


t0 = time.perf_counter()
try:
    sandbox = Sandbox(backend="hyperlight-js")
except ImportError as exc:
    raise SystemExit(
        "This example requires the HyperlightJS backend. "
        "Install hyperlight-sandbox[hyperlight_js] or run `just python-build`."
    ) from exc
print(f"[timer]  Sandbox created (lazy): {(time.perf_counter() - t0) * 1000:.1f}ms")

sandbox.register_tool("add", lambda a=0, b=0: a + b)
sandbox.register_tool("multiply", lambda a=0, b=0: a * b)
sandbox.register_tool("greet", lambda name="world": f"Hello, {name}!")
sandbox.register_tool("lookup", lambda key="": {"api_key": "sk-demo", "model": "gpt-4"}.get(key, "not found"))

print("\n--- Test 1: Basic execution ---")
result = timed_run(sandbox, 'console.log("hello from js sandbox!");', "first run")
print(f"stdout: {result.stdout!r}")
print(f"success: {result.success}")
assert result.success, result.stderr
assert "hello from js sandbox!" in result.stdout, (result.stdout, result.stderr)

print("\n--- Test 2: Tool dispatch ---")
result = timed_run(
    sandbox,
    """
const sum = call_tool("add", { a: 3, b: 4 });
const greeting = call_tool("greet", { name: "James" });
console.log(`3 + 4 = ${sum}`);
console.log(greeting);
""",
    "tool dispatch",
)
print(f"stdout: {result.stdout!r}")
assert result.success, result.stderr
assert "3 + 4 = 7" in result.stdout, (result.stdout, result.stderr)
assert "Hello, James!" in result.stdout, (result.stdout, result.stderr)

print("\n--- Test 3: Snapshot/restore ---")
snap = sandbox.snapshot()
before = timed_run(
    sandbox,
    """
globalThis.counter = (globalThis.counter ?? 0) + 1;
console.log(`counter = ${globalThis.counter}`);
""",
    "before restore",
)
print(f"before restore: {before.stdout!r}")
assert before.success, before.stderr
assert "counter = 1" in before.stdout, (before.stdout, before.stderr)
sandbox.restore(snap)
after = timed_run(
    sandbox,
    'console.log(`counter after restore = ${globalThis.counter ?? "missing"}`);',
    "after restore",
)
print(f"after restore: {after.stdout!r}")
assert after.success, after.stderr
assert "counter after restore = missing" in after.stdout, (after.stdout, after.stderr)

# Test 4: Complex multi-step computation
print("\n--- Test 4: Complex multi-step computation ---")
result = timed_run(
    sandbox,
    """
const data = [];
for (let i = 0; i < 5; i++) {
    data.push(call_tool('multiply', { a: i, b: i }));
}
const firstThree = data.slice(0, 3).reduce((a, b) => a + b, 0);
const lastTwo = data.slice(3).reduce((a, b) => a + b, 0);
const total = call_tool('add', { a: firstThree, b: lastTwo });
console.log('Squares: [' + data.join(', ') + ']');
console.log('Total: ' + total);
""",
    "complex computation",
)
print(f"stdout: {result.stdout!r}")
assert result.success, result.stderr
assert "Total: 30" in result.stdout, (result.stdout, result.stderr)

# Test 5: Nested tool calls
print("\n--- Test 5: Nested tool calls ---")
result = timed_run(
    sandbox,
    """
const nested = call_tool('multiply', { a: call_tool('add', { a: 3, b: 4 }), b: 5 });
console.log('(3 + 4) * 5 = ' + nested);

const deep = call_tool('add', {
    a: call_tool('multiply', { a: 2, b: 3 }),
    b: call_tool('multiply', { a: 4, b: 5 }),
});
console.log('(2 * 3) + (4 * 5) = ' + deep);

const greeting = call_tool('greet', { name: call_tool('lookup', { key: 'model' }) });
console.log('Greeting with lookup: ' + greeting);
""",
    "nested tools",
)
print(f"stdout: {result.stdout!r}")
assert result.success, result.stderr
lines = result.stdout.strip().splitlines()
assert "35" in lines[0], f"Expected 35, got: {lines[0]}"
assert "26" in lines[1], f"Expected 26, got: {lines[1]}"
assert "gpt-4" in lines[2], f"Expected gpt-4, got: {lines[2]}"

print("\n[ok] JavaScript basics example passed!")
