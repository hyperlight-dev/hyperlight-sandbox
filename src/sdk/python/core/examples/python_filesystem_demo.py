"""Filesystem capabilities demo -- exercises all input/output combinations."""

import os
import tempfile

from hyperlight_sandbox import Sandbox


def separator(label: str) -> None:
    print(f"\n-- {label} --")


# -- Test 1: No filesystem --------------------------------------------
separator("Test 1: No filesystem")
sandbox = Sandbox()
result = sandbox.run("print('no filesystem needed')")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "no filesystem needed" in result.stdout
outputs = sandbox.get_output_files()
assert len(outputs) == 0
print("OK: sandbox runs without any filesystem")

# -- Test 2: Input only -----------------------------------------------
separator("Test 2: Input only")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "greeting.txt"), "w") as f:
    f.write("hello from host")

sandbox = Sandbox(input_dir=input_dir)
result = sandbox.run("""
with open('/input/greeting.txt') as f:
    print(f'content: {f.read()}')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "hello from host" in result.stdout
outputs = sandbox.get_output_files()
assert len(outputs) == 0
print("OK: guest reads host-provided input")

# -- Test 3: Temp output only -----------------------------------------
separator("Test 3: Temp output only")
sandbox = Sandbox(temp_output=True)
result = sandbox.run("""
with open('/output/result.txt', 'w') as f:
    f.write('computed result')
print('wrote output')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
output_dir = sandbox.output_path()
output_files = sandbox.get_output_files()
assert "result.txt" in output_files
with open(os.path.join(output_dir, "result.txt"), "rb") as f:
    assert f.read() == b"computed result"
print("OK: guest writes to temp output, host collects files")

# -- Test 4: Input + temp output --------------------------------------
separator("Test 4: Input + temp output")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "data.json"), "w") as f:
    f.write('{"value": 42}')

sandbox = Sandbox(input_dir=input_dir, temp_output=True)
result = sandbox.run("""
import json
with open('/input/data.json') as f:
    data = json.load(f)
result = data['value'] * 2
with open('/output/doubled.txt', 'w') as f:
    f.write(str(result))
print(f'doubled: {result}')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "doubled: 84" in result.stdout
output_dir = sandbox.output_path()
output_files = sandbox.get_output_files()
assert "doubled.txt" in output_files
with open(os.path.join(output_dir, "doubled.txt"), "rb") as f:
    assert f.read() == b"84"
print("OK: guest reads input, writes output, host collects")

# -- Test 5: Explicit output dir --------------------------------------
separator("Test 5: Explicit output dir")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
output_dir = tempfile.mkdtemp(prefix="sandbox-output-")
with open(os.path.join(input_dir, "source.txt"), "w") as f:
    f.write("transform me")

sandbox = Sandbox(input_dir=input_dir, output_dir=output_dir)
result = sandbox.run("""
with open('/input/source.txt') as f:
    text = f.read()
with open('/output/upper.txt', 'w') as f:
    f.write(text.upper())
print(f'transformed: {text.upper()}')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "TRANSFORM ME" in result.stdout
# Verify file exists on host filesystem
with open(os.path.join(output_dir, "upper.txt")) as f:
    assert f.read() == "TRANSFORM ME"
print("OK: output written to explicit dir, visible on host")

# -- Test 6: Output is ephemeral --------------------------------------
separator("Test 6: Output is ephemeral (wiped between runs)")
sandbox = Sandbox(temp_output=True)

result = sandbox.run("""
with open('/output/run1.txt', 'w') as f:
    f.write('first run')
""")
outputs = sandbox.get_output_files()
assert "run1.txt" in outputs, "run1.txt should exist after first run"

result = sandbox.run("""
with open('/output/run2.txt', 'w') as f:
    f.write('second run')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
outputs = sandbox.get_output_files()
assert "run1.txt" not in outputs, "run1.txt should be wiped"
assert "run2.txt" in outputs
print("OK: output wiped between runs")

# -- Test 7: Input is read-only ---------------------------------------
separator("Test 7: Input is read-only")
input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
with open(os.path.join(input_dir, "readonly.txt"), "w") as f:
    f.write("do not modify")

sandbox = Sandbox(input_dir=input_dir)
result = sandbox.run("""
try:
    with open('/input/readonly.txt', 'w') as f:
        f.write('hacked')
    print('FAIL: write succeeded')
except (OSError, PermissionError) as e:
    print(f'OK: write blocked: {e}')
""")
assert result.exit_code == 0, f"stderr: {result.stderr}"
assert "OK: write blocked" in result.stdout
with open(os.path.join(input_dir, "readonly.txt")) as f:
    assert f.read() == "do not modify"
print("OK: guest cannot write to input")

print("\n[ok] All filesystem tests passed!")
