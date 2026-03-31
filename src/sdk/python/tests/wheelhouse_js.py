from hyperlight_sandbox import Sandbox

sandbox = Sandbox(backend="hyperlight-js")
result = sandbox.run('console.log("wheelhouse install ok")')
assert result.success, result.stderr
print(result.stdout.strip())
