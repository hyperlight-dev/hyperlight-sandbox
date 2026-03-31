"""Hyperlight Sandbox — Python basics example.

Exercises: basic execution, tool dispatch (sync + async), sandbox reuse,
snapshot/restore, complex computation, and nested tool calls.

File I/O tests live in python_filesystem_demo.py.
Network tests live in python_network_demo.py.
"""

import asyncio
import time

from hyperlight_sandbox import Sandbox


def timed_run(sandbox, code, label="run"):
    """Run code in sandbox and print timing."""
    start = time.perf_counter()
    result = sandbox.run(code)
    elapsed_ms = (time.perf_counter() - start) * 1000
    print(f"⏱️  {label}: {elapsed_ms:.1f}ms")
    return result


# Create sandbox using the packaged python_guest artifact
t0 = time.perf_counter()
try:
    sandbox = Sandbox(backend="wasm", module="python_guest.path")
except ImportError as exc:
    raise SystemExit(
        "This example requires the Wasm backend and packaged Python guest package. "
        "Install hyperlight-sandbox[wasm,python_guest] or run `just python-build`."
    ) from exc
print(f"⏱️  Sandbox created (lazy): {(time.perf_counter() - t0) * 1000:.1f}ms")

# Register host tools before first run()
sandbox.register_tool("add", lambda a=0, b=0: a + b)
sandbox.register_tool("multiply", lambda a=0, b=0: a * b)
sandbox.register_tool("greet", lambda name="world": f"Hello, {name}!")
sandbox.register_tool("lookup", lambda key="": {"api_key": "sk-demo", "model": "gpt-4"}.get(key, "not found"))


# Async functions work too — no wrapping needed
async def async_multiply(a: float, b: float):
    await asyncio.sleep(0.5)  # simulate async I/O
    return a * b


sandbox.register_tool("async_multiply", async_multiply)

# Test 1: Basic code execution (first run triggers sandbox init)
print("\n--- Test 1: Basic execution (includes sandbox init) ---")
result = timed_run(sandbox, 'print("hello from python SDK!")', "first run (cold)")
print(f"stdout: {result.stdout!r}")
print(f"success: {result.success}")

# Test 2: Tool dispatch via call_tool() — sync and async
print("\n--- Test 2: Tool dispatch (sync + async) ---")
result = timed_run(
    sandbox,
    """
import time
result = call_tool('add', a=3, b=4)
greeting = call_tool('greet', name='James')
t0 = time.time()
product = call_tool('async_multiply', a=6, b=7)
elapsed = time.time() - t0
print(f"3 + 4 = {result}")
print(f"{greeting}")
print(f"6 * 7 = {product}  (async tool, slept {elapsed:.1f}s)")
try:
    call_tool('nonexistent', x=1)
except RuntimeError as e:
    print(f"Caught error: {e}")
print("All tool tests passed!")
""",
    "tool dispatch",
)
print(f"stdout: {result.stdout!r}")
print(f"success: {result.success}")

# Test 3: Sandbox reuse
print("\n--- Test 3: Sandbox reuse ---")
result = timed_run(sandbox, 'print("second run works!")', "reuse (warm)")
print(f"stdout: {result.stdout!r}")
print(f"success: {result.success}")

# Test 4: Snapshot/restore
print("\n--- Test 4: Snapshot/restore ---")
t0 = time.perf_counter()
snap = sandbox.snapshot()
print(f"⏱️  snapshot: {(time.perf_counter() - t0) * 1000:.1f}ms")

result1 = timed_run(sandbox, 'x = 42; print(f"x = {x}")', "pre-restore run")
print(f"Before restore: {result1.stdout!r}")

t0 = time.perf_counter()
sandbox.restore(snap)
print(f"⏱️  restore: {(time.perf_counter() - t0) * 1000:.1f}ms")

result2 = timed_run(
    sandbox,
    """
try:
    print(f"x = {x}")
except NameError:
    print("x is not defined (state was rolled back)")
""",
    "post-restore run",
)
print(f"After restore: {result2.stdout!r}")

# Test 5: Complex multi-step computation
print("\n--- Test 5: Complex multi-step computation ---")
result = timed_run(
    sandbox,
    """
data = []
for i in range(5):
    val = call_tool('multiply', a=i, b=i)
    data.append(val)
total = call_tool('add', a=sum(data[:3]), b=sum(data[3:]))
print(f"Squares: {data}")
print(f"Total: {total}")
""",
    "complex computation",
)
print(f"stdout: {result.stdout!r}")
assert result.success

# Test 6: Nested tool calls
print("\n--- Test 6: Nested tool calls ---")
result = timed_run(
    sandbox,
    """
# (3 + 4) * 5 = 35
nested = call_tool('multiply', a=call_tool('add', a=3, b=4), b=5)
print(f"(3 + 4) * 5 = {nested}")

# (2 * 3) + (4 * 5) = 26
deep = call_tool('add',
    a=call_tool('multiply', a=2, b=3),
    b=call_tool('multiply', a=4, b=5),
)
print(f"(2 * 3) + (4 * 5) = {deep}")

greeting = call_tool('greet', name=call_tool('lookup', key='model'))
print(f"Greeting with lookup: {greeting}")
""",
    "nested tools",
)
print(f"stdout: {result.stdout!r}")
assert result.success
lines = result.stdout.strip().splitlines()
assert "35" in lines[0], f"Expected 35, got: {lines[0]}"
assert "26" in lines[1], f"Expected 26, got: {lines[1]}"
assert "gpt-4" in lines[2], f"Expected gpt-4 in greeting, got: {lines[2]}"

print("\n✅ All tests passed!")
