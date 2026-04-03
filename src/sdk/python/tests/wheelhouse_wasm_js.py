from hyperlight_sandbox import Sandbox

sandbox = Sandbox(backend="wasm", module="javascript_guest.path")
result = sandbox.run('console.log("wheelhouse wasm javascript install ok")')
assert result.success, result.stderr
print(result.stdout.strip())
