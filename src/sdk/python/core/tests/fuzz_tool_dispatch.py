#!/usr/bin/env python3
"""Atheris coverage-guided fuzz test for tool argument dispatch.

Uses libFuzzer via atheris to generate random byte sequences, converts them
to JSON-like arguments, and fires them at tool dispatch through a real
Wasm sandbox. The invariant: the host process must never crash.

Usage:
    # Run for 60 seconds (default):
    uv run python src/sdk/python/core/tests/fuzz_tool_dispatch.py

    # Run for N seconds:
    uv run python src/sdk/python/core/tests/fuzz_tool_dispatch.py -max_total_time=120

    # Run until crash (no time limit):
    uv run python src/sdk/python/core/tests/fuzz_tool_dispatch.py -max_total_time=0

    # With a corpus directory (for reproducibility):
    uv run python src/sdk/python/core/tests/fuzz_tool_dispatch.py corpus/
"""

import sys

import atheris

# Lazy-init to avoid paying sandbox startup cost during atheris setup.
_sandboxes = {}

# Execution counters
_stats = {
    "typed_runs": 0,
    "typed_schema_rejected": 0,
    "typed_ok": 0,
    "untyped_runs": 0,
    "untyped_ok": 0,
    "raw_runs": 0,
    "sandbox_recreated": 0,
    "guest_errors": 0,
}


def _get_sandbox(key):
    """Get or create a sandbox. Re-creates if poisoned."""
    from hyperlight_sandbox import Sandbox

    if key not in _sandboxes:
        s = Sandbox(backend="wasm")
        if key == "typed":
            s.register_tool("add", lambda a=0, b=0: a + b)
            s.register_tool("greet", lambda name="world": f"Hello, {name}!")
            s.register_tool("check", lambda flag=True: flag)
            s.register_tool("listify", lambda items=[]: items)
            s.register_tool("echo_dict", lambda data={}: data)
        elif key == "untyped":
            s.register_tool("raw", lambda x: f"got: {x}")
            s.register_tool("multi", lambda a, b: f"{a}-{b}")
        elif key == "kwargs":
            s.register_tool("kw", lambda **kw: kw)
        # Warm up
        try:
            s.run("None")
        except Exception:
            pass
        _sandboxes[key] = s
    return _sandboxes[key]


def _reset_sandbox(key):
    """Discard a poisoned sandbox so next call recreates it."""
    _sandboxes.pop(key, None)
    _stats["sandbox_recreated"] += 1


def _fuzz_typed_dispatch(data):
    """Fuzz typed tools with random argument values."""
    fdp = atheris.FuzzedDataProvider(data)

    tools = ["add", "greet", "check", "listify", "echo_dict"]
    tool_name = fdp.PickValueInList(tools)

    # Generate random argument names and values
    num_args = fdp.ConsumeIntInRange(0, 5)
    args = {}
    # Use safe identifier-like keys to avoid generating invalid Python syntax
    safe_keys = ["a", "b", "name", "flag", "data", "items", "x", "y", "z", "val"]
    for i in range(num_args):
        key = safe_keys[i % len(safe_keys)]
        val_type = fdp.ConsumeIntInRange(0, 7)
        if val_type == 0:
            args[key] = None
        elif val_type == 1:
            args[key] = fdp.ConsumeBool()
        elif val_type == 2:
            args[key] = fdp.ConsumeInt(8)
        elif val_type == 3:
            args[key] = fdp.ConsumeRegularFloat()
        elif val_type == 4:
            args[key] = fdp.ConsumeUnicodeNoSurrogates(fdp.ConsumeIntInRange(0, 100))
        elif val_type == 5:
            args[key] = [fdp.ConsumeInt(4) for _ in range(fdp.ConsumeIntInRange(0, 5))]
        elif val_type == 6:
            args[key] = {fdp.ConsumeUnicodeNoSurrogates(5): fdp.ConsumeInt(4)}
        else:
            args[key] = fdp.ConsumeUnicodeNoSurrogates(fdp.ConsumeIntInRange(0, 1000))

    # Build call_tool invocation
    args_str = ", ".join(f"{k}={v!r}" for k, v in args.items())
    code = f"""
try:
    call_tool('{tool_name}', {args_str})
except Exception:
    pass
"""

    sandbox = _get_sandbox("typed")
    try:
        result = sandbox.run(code)
        _stats["typed_runs"] += 1
        # Host survived — the only thing that matters
        if result.exit_code == 0:
            _stats["typed_ok"] += 1
        else:
            _stats["guest_errors"] += 1
            _stats["typed_schema_rejected"] += 1
    except RuntimeError as e:
        _stats["typed_runs"] += 1
        _stats["guest_errors"] += 1
        err = str(e)
        if "poisoned" in err or "GeneralProtectionFault" in err:
            # Sandbox crashed — not a host crash, but recreate it
            _reset_sandbox("typed")
        # Host process is still alive — that's the invariant


def _fuzz_untyped_dispatch(data):
    """Fuzz untyped tools — these accept any type."""
    fdp = atheris.FuzzedDataProvider(data)

    tool_name = fdp.PickValueInList(["raw", "multi"])

    num_args = fdp.ConsumeIntInRange(0, 4)
    args = {}
    arg_names = ["x", "a", "b", "extra"]
    for i in range(num_args):
        key = arg_names[i] if i < len(arg_names) else f"arg{i}"
        val_type = fdp.ConsumeIntInRange(0, 7)
        if val_type == 0:
            args[key] = None
        elif val_type == 1:
            args[key] = fdp.ConsumeBool()
        elif val_type == 2:
            args[key] = fdp.ConsumeInt(8)
        elif val_type == 3:
            args[key] = fdp.ConsumeRegularFloat()
        elif val_type == 4:
            args[key] = fdp.ConsumeUnicodeNoSurrogates(fdp.ConsumeIntInRange(0, 100))
        elif val_type == 5:
            args[key] = [fdp.ConsumeInt(4) for _ in range(fdp.ConsumeIntInRange(0, 5))]
        elif val_type == 6:
            # Nested dict
            inner = {}
            for _ in range(fdp.ConsumeIntInRange(0, 3)):
                inner[fdp.ConsumeUnicodeNoSurrogates(5)] = fdp.ConsumeUnicodeNoSurrogates(10)
            args[key] = inner
        else:
            args[key] = fdp.ConsumeUnicodeNoSurrogates(fdp.ConsumeIntInRange(0, 500))

    args_str = ", ".join(f"{k}={v!r}" for k, v in args.items())
    code = f"""
try:
    call_tool('{tool_name}', {args_str})
except Exception:
    pass
"""

    sandbox = _get_sandbox("untyped")
    try:
        result = sandbox.run(code)
        _stats["untyped_runs"] += 1
        if result.exit_code == 0:
            _stats["untyped_ok"] += 1
        else:
            _stats["guest_errors"] += 1
    except RuntimeError as e:
        _stats["untyped_runs"] += 1
        _stats["guest_errors"] += 1
        if "poisoned" in str(e):
            _reset_sandbox("untyped")


def _fuzz_raw_json(data):
    """Fuzz with completely raw bytes interpreted as a JSON string.

    This tests the worst case: the guest sends arbitrary bytes as the
    tool request JSON. The host must never crash.
    """
    fdp = atheris.FuzzedDataProvider(data)

    # Generate a raw string that might or might not be valid JSON
    raw = fdp.ConsumeUnicodeNoSurrogates(fdp.ConsumeIntInRange(0, 2000))

    # Escape for Python string literal inside guest code
    escaped = raw.replace("\\", "\\\\").replace("'", "\\'").replace("\n", "\\n").replace("\r", "\\r")

    code = f"""
try:
    call_tool('add', a='{escaped}')
except Exception:
    pass
"""

    sandbox = _get_sandbox("typed")
    try:
        result = sandbox.run(code)
        _stats["raw_runs"] += 1
        if result.exit_code == 0:
            _stats["typed_ok"] += 1
        else:
            _stats["guest_errors"] += 1
    except RuntimeError as e:
        _stats["raw_runs"] += 1
        _stats["guest_errors"] += 1
        if "poisoned" in str(e):
            _reset_sandbox("typed")


def TestOneInput(data):
    """Main fuzz target — dispatches to sub-fuzzers."""
    if len(data) < 2:
        return

    # Use first byte to pick which fuzzer to run
    selector = data[0] % 3
    rest = data[1:]

    if selector == 0:
        _fuzz_typed_dispatch(rest)
    elif selector == 1:
        _fuzz_untyped_dispatch(rest)
    else:
        _fuzz_raw_json(rest)

    # Print stats every 1000 guest executions
    total = _stats["typed_runs"] + _stats["untyped_runs"] + _stats["raw_runs"]
    if total > 0 and total % 1000 == 0:
        sys.stderr.write(
            f"  [fuzz] {total} guest runs | "
            f"typed: {_stats['typed_runs']} ({_stats['typed_ok']} ok) | "
            f"untyped: {_stats['untyped_runs']} ({_stats['untyped_ok']} ok) | "
            f"raw: {_stats['raw_runs']} | "
            f"errors: {_stats['guest_errors']} | "
            f"recreated: {_stats['sandbox_recreated']}\n"
        )
        sys.stderr.flush()


def main():
    # Default: run for 60 seconds
    if not any(arg.startswith("-max_total_time") for arg in sys.argv[1:]):
        sys.argv.append("-max_total_time=60")

    # Suppress atheris's own output flooding
    if not any(arg.startswith("-print_final_stats") for arg in sys.argv[1:]):
        sys.argv.append("-print_final_stats=1")

    print("🔀 Starting atheris fuzz test for tool dispatch...")
    print(f"   Args: {sys.argv[1:]}")
    print("   Invariant: host process must never crash")
    print("   Stats printed every 1000 guest executions")
    print(flush=True)

    atheris.Setup(sys.argv, TestOneInput)
    atheris.Fuzz()


if __name__ == "__main__":
    main()
