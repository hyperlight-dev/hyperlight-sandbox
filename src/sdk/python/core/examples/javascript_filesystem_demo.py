"""Filesystem demo for the Hyperlight JS backend (Python SDK)."""

import os
import tempfile

from hyperlight_sandbox import Sandbox


def separator(label: str) -> None:
    print(f"\n── {label} ──")


# ── Test 1: No filesystem ────────────────────────────────────────────
separator("Test 1: No filesystem")
sandbox = Sandbox(backend="hyperlight-js")
result = sandbox.run("console.log('no filesystem needed');")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "no filesystem needed" in result.stdout
outputs = sandbox.get_output_files()
assert len(outputs) == 0
print("OK: runs without filesystem")

# ── Test 2: Input only ───────────────────────────────────────────────
separator("Test 2: Input only")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "greeting.txt"), "w") as f:
    f.write("hello from host")

sandbox = Sandbox(backend="hyperlight-js", input_dir=input_dir)
result = sandbox.run("const t = read_file('/input/greeting.txt'); console.log('content: ' + t);")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "hello from host" in result.stdout
print("OK: guest reads input")

# ── Test 3: Temp output only ─────────────────────────────────────────
separator("Test 3: Temp output only")
sandbox = Sandbox(backend="hyperlight-js", temp_output=True)
result = sandbox.run("write_file('/output/result.txt', 'js output'); console.log('wrote');")
assert result.exit_code == 0, f"stderr: {result.stderr}"
output_dir = sandbox.output_path()
output_files = sandbox.get_output_files()
assert "result.txt" in output_files
with open(os.path.join(output_dir, "result.txt"), "rb") as f:
    assert f.read() == b"js output"
print("OK: guest writes to temp output")

# ── Test 4: Input + temp output ──────────────────────────────────────
separator("Test 4: Input + temp output")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "data.json"), "w") as f:
    f.write('{"n": 21}')

sandbox = Sandbox(backend="hyperlight-js", input_dir=input_dir, temp_output=True)
result = sandbox.run("""
const data = JSON.parse(read_file('/input/data.json'));
write_file('/output/doubled.txt', String(data.n * 2));
console.log('doubled: ' + data.n * 2);
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "doubled: 42" in result.stdout
output_dir = sandbox.output_path()
output_files = sandbox.get_output_files()
assert "doubled.txt" in output_files
with open(os.path.join(output_dir, "doubled.txt"), "rb") as f:
    assert f.read() == b"42"
print("OK: reads input, writes output")

# ── Test 5: Explicit output dir ──────────────────────────────────────
separator("Test 5: Explicit output dir")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
output_dir = tempfile.mkdtemp(prefix="sandbox-output-")
with open(os.path.join(input_dir, "msg.txt"), "w") as f:
    f.write("shout")

sandbox = Sandbox(backend="hyperlight-js", input_dir=input_dir, output_dir=output_dir)
result = sandbox.run("""
const text = read_file('/input/msg.txt');
write_file('/output/upper.txt', text.toUpperCase());
console.log('done');
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
with open(os.path.join(output_dir, "upper.txt")) as f:
    assert f.read() == "SHOUT"
print("OK: output visible on host filesystem")

# ── Test 6: Output wiped between runs ────────────────────────────────
separator("Test 6: Output is ephemeral")
sandbox = Sandbox(backend="hyperlight-js", temp_output=True)
result = sandbox.run("write_file('/output/run1.txt', 'first');")
outputs = sandbox.get_output_files()
assert "run1.txt" in outputs, "run1.txt should exist"

result = sandbox.run("write_file('/output/run2.txt', 'second');")
outputs = sandbox.get_output_files()
assert "run1.txt" not in outputs, "run1 should be wiped"
assert "run2.txt" in outputs
print("OK: output wiped between runs")

# ── Test 7: Input is read-only ───────────────────────────────────────
separator("Test 7: Input is read-only")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "readonly.txt"), "w") as f:
    f.write("do not modify")

sandbox = Sandbox(backend="hyperlight-js", input_dir=input_dir)
result = sandbox.run("""
try {
    write_file('/input/readonly.txt', 'hacked');
    console.log('FAIL: write succeeded');
} catch (e) {
    console.log('OK: write blocked: ' + e);
}
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "OK: write blocked" in result.stdout
with open(os.path.join(input_dir, "readonly.txt")) as f:
    assert f.read() == "do not modify"
print("OK: guest cannot write to input")

print("\n[ok] All JS filesystem tests passed!")
