"""
Benchmark: Hyperlight Sandbox timing across all backends.

Creates a fresh sandbox for each test to get clean measurements.
Backends tested: Wasm+Python, Wasm+JavaScript, HyperlightJS.

Each test runs multiple iterations (configurable via COLD_ROUNDS / WARM_ROUNDS)
and reports min / avg / max.
"""

import os
import tempfile
import time

# How many iterations per test
COLD_ROUNDS = 5  # tests that create a new sandbox each time
WARM_ROUNDS = 10  # tests that reuse an existing sandbox

# ── Helpers ──────────────────────────────────────────────────────────


def measure_n(fn, n):
    """Run fn n times, return (last_result, stats_dict)."""
    times = []
    result = None
    for _ in range(n):
        start = time.perf_counter()
        result = fn()
        ms = (time.perf_counter() - start) * 1000
        times.append(ms)
    return result, {"min": min(times), "avg": sum(times) / len(times), "max": max(times)}


def fmt(stats):
    """Format stats as 'avg ms  (min=…, max=…)'."""
    return f"{stats['avg']:>7.1f} ms  (min={stats['min']:.1f}, max={stats['max']:.1f})"


def fmt_short(stats):
    """Short format for summary table: 'avg ms'."""
    return f"{stats['avg']:.1f} ms"


def run_suite(name, backend, module=None, lang="python"):
    """Run all benchmarks for one backend configuration."""
    from hyperlight_sandbox import Sandbox

    print(f"\n{'═' * 70}")
    print(f"  {name}  ({COLD_ROUNDS} cold / {WARM_ROUNDS} warm rounds)")
    print(f"{'═' * 70}")

    hello = 'print("hello")' if lang == "python" else 'console.log("hello")'
    tool_code = (
        "result = call_tool('add', a=3, b=4)\nprint(result)"
        if lang == "python"
        else 'const result = call_tool("add", {a: 3, b: 4}); console.log(result);'
    )
    file_code = None
    if lang == "python":
        file_code = 'with open("/input/data.json") as f:\n    import json; print(json.load(f)["name"])'
    elif backend == "hyperlight-js":
        file_code = 'const data = JSON.parse(readFile("/input/data.json"));\nconsole.log(data.name);'

    kwargs = {"backend": backend}
    if module:
        kwargs["module"] = module

    results = {}

    # ── 1. Cold start: create + first run ────────────────────────
    def cold_start():
        s = Sandbox(**kwargs)
        r = s.run(hello)
        assert r.success, f"cold start failed: {r.stderr}"
        return s

    sandbox, stats = measure_n(cold_start, COLD_ROUNDS)
    results["Cold start (create + first run)"] = stats
    print(f"  Cold start (create + first run): {fmt(stats)}")

    # ── 2. Warm run (no restore) ───────────────────────────────
    _, stats = measure_n(lambda: sandbox.run(hello), WARM_ROUNDS)
    results["Warm run (no restore)"] = stats
    print(f"  Warm run (no restore):           {fmt(stats)}")

    # ── 3. Cold start + tool dispatch ────────────────────────────
    def with_tools():
        s = Sandbox(**kwargs)
        s.register_tool("add", lambda a=0, b=0: a + b)
        r = s.run(tool_code)
        assert r.success, f"tool dispatch failed: {r.stderr}"
        return r

    _, stats = measure_n(with_tools, COLD_ROUNDS)
    results["Cold start + tool dispatch"] = stats
    print(f"  Cold start + tool dispatch:      {fmt(stats)}")

    # ── 4. Warm tool dispatch (no restore) ────────────────────────
    warm_tool_sandbox = Sandbox(**kwargs)
    warm_tool_sandbox.register_tool("add", lambda a=0, b=0: a + b)
    warm_tool_sandbox.run(hello)  # warm up
    _, stats = measure_n(lambda: warm_tool_sandbox.run(tool_code), WARM_ROUNDS)
    results["Warm tool dispatch (no restore)"] = stats
    print(f"  Warm tool dispatch (no restore): {fmt(stats)}")

    # ── 5. File I/O ──────────────────────────────────────────────
    if file_code:

        def with_files():
            input_dir = tempfile.mkdtemp(prefix="sandbox-bench-")
            with open(os.path.join(input_dir, "data.json"), "wb") as f:
                f.write(b'{"name": "Alice"}')
            s = Sandbox(input_dir=input_dir, **kwargs)
            r = s.run(file_code)
            assert r.success, f"file I/O failed: {r.stderr}"
            return r

        _, stats = measure_n(with_files, COLD_ROUNDS)
        results["Cold start + file I/O"] = stats
        print(f"  Cold start + file I/O:           {fmt(stats)}")

        _, stats = measure_n(lambda: sandbox.run(file_code), WARM_ROUNDS)
        results["Warm file I/O (no restore)"] = stats
        print(f"  Warm file I/O (no restore):      {fmt(stats)}")
    else:
        results["Cold start + file I/O"] = None
        results["Warm file I/O (no restore)"] = None
        print("  File I/O:                        n/a (not supported)")

    # ── 6. Snapshot (create sandbox, run, then time just the snapshot) ──
    def one_snapshot():
        s = Sandbox(**kwargs)
        s.register_tool("add", lambda a=0, b=0: a + b)
        s.run(hello)
        start = time.perf_counter()
        snap = s.snapshot()
        ms = (time.perf_counter() - start) * 1000
        return snap, ms

    snap_times = []
    for _ in range(COLD_ROUNDS):
        _, ms = one_snapshot()
        snap_times.append(ms)
    stats = {"min": min(snap_times), "avg": sum(snap_times) / len(snap_times), "max": max(snap_times)}
    results["Snapshot"] = stats
    print(f"  Snapshot:                        {fmt(stats)}")

    # ── 7. Restore (create sandbox, snapshot, then time just the restore) ──
    def one_restore():
        s = Sandbox(**kwargs)
        s.register_tool("add", lambda a=0, b=0: a + b)
        s.run(hello)
        sn = s.snapshot()
        start = time.perf_counter()
        s.restore(sn)
        ms = (time.perf_counter() - start) * 1000
        return s, ms

    restore_times = []
    for _ in range(COLD_ROUNDS):
        _, ms = one_restore()
        restore_times.append(ms)
    stats = {"min": min(restore_times), "avg": sum(restore_times) / len(restore_times), "max": max(restore_times)}
    results["Restore"] = stats
    print(f"  Restore:                         {fmt(stats)}")

    # ── 8. Restore + run ─────────────────────────────────────────
    snap_sandbox = Sandbox(**kwargs)
    snap_sandbox.register_tool("add", lambda a=0, b=0: a + b)
    snap_sandbox.run(hello)
    snap = snap_sandbox.snapshot()
    snap_sandbox.restore(snap)  # warm-up restore (first restore allocates pages)

    def restore_and_run():
        snap_sandbox.restore(snap)
        return snap_sandbox.run(hello)

    _, stats = measure_n(restore_and_run, WARM_ROUNDS)
    results["Restore + run"] = stats
    print(f"  Restore + run:                   {fmt(stats)}")

    # ── 9. Restore + tool dispatch ───────────────────────────────
    def restore_and_tool():
        snap_sandbox.restore(snap)
        return snap_sandbox.run(tool_code)

    _, stats = measure_n(restore_and_tool, WARM_ROUNDS)
    results["Restore + tool dispatch"] = stats
    print(f"  Restore + tool dispatch:         {fmt(stats)}")

    return results


# ── Main ─────────────────────────────────────────────────────────────


def main():
    all_results = {}

    # Python in Wasm
    try:
        r = run_suite("Wasm + Python", backend="wasm", module="python_guest.path", lang="python")
        all_results["Wasm + Python"] = r
    except Exception as e:
        print(f"  ⚠️  Skipped: {e}")

    # JavaScript in Wasm
    try:
        r = run_suite("Wasm + JavaScript", backend="wasm", module="javascript_guest.path", lang="javascript")
        all_results["Wasm + JavaScript"] = r
    except Exception as e:
        print(f"  ⚠️  Skipped: {e}")

    # HyperlightJS
    try:
        r = run_suite("HyperlightJS", backend="hyperlight-js", lang="javascript")
        all_results["HyperlightJS"] = r
    except Exception as e:
        print(f"  ⚠️  Skipped: {e}")

    # ── Summary table (averages) ─────────────────────────────────
    if all_results:
        print(f"\n{'═' * 90}")
        print("  Summary (avg ms)")
        print(f"{'═' * 90}")

        backends = list(all_results.keys())
        steps = list(next(iter(all_results.values())).keys())

        header = f"{'Step':<35}" + "".join(f"{b:>18}" for b in backends)
        print(header)
        print("─" * len(header))
        for step in steps:
            row = f"{step:<35}"
            for b in backends:
                s = all_results[b].get(step)
                row += f"{fmt_short(s) if s else 'n/a':>18}"
            print(row)

    print(f"\n{'═' * 90}")
    print(f"  Done!  ({COLD_ROUNDS} cold / {WARM_ROUNDS} warm rounds per test)")
    print(f"{'═' * 90}")


if __name__ == "__main__":
    main()
