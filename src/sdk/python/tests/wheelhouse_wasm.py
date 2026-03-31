from hyperlight_sandbox import Sandbox
from hyperlight_sandbox._module_resolver import resolve_module_path

path = resolve_module_path(module="python_guest.path")
print(path)
sandbox = Sandbox(backend="wasm", module="python_guest.path")
result = sandbox.run('print("wheelhouse install ok")')
assert result.success, result.stderr
print(result.stdout.strip())
