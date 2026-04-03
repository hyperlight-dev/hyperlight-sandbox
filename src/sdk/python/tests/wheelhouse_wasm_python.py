from hyperlight_sandbox import Sandbox

sandbox = Sandbox(backend="wasm", module="python_guest.path")
result = sandbox.run('print("wheelhouse wasm python install ok")')
assert result.success, result.stderr
print(result.stdout.strip())
