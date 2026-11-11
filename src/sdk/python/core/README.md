# hyperlight-sandbox

Python API package for running code inside Hyperlight sandboxes with separately
installable backends and packaged guest packages.

## Quick Start

```python
from hyperlight_sandbox import Sandbox

sandbox = Sandbox(backend="wasm", module="python_guest.path")
sandbox.register_tool("add", lambda a=0, b=0: a + b)

result = sandbox.run("""
result = call_tool('add', a=3, b=4)
print(result)
""")
print(result.stdout)  # "7\n"
```

Install the local repo packages for development with:

```bash
uv sync          # installs core + guest packages via workspace
just python-build  # builds maturin backends
```

Packaged guest packages expose importable module references such as
`python_guest.path` and `javascript_guest.path`. The API resolves those to the
packaged `.aot` artifact automatically.

Example dependency sets:

- `Sandbox(backend="wasm", module="python_guest.path")` requires `hyperlight-sandbox[wasm,python_guest]`
- `Sandbox(backend="wasm", module="javascript_guest.path")` requires `hyperlight-sandbox[wasm,javascript_guest]`
- `Sandbox(backend="hyperlight-js")` requires `hyperlight-sandbox[hyperlight_js]`

Use `Sandbox(backend="wasm", module="javascript_guest.path")` to run the
packaged JavaScript Wasm guest package.

Use `Sandbox(backend="hyperlight-js")` to run the separate HyperlightJS
backend package.

## macOS

Apple Silicon wheels provide developer-only support for macOS 13 and newer.
Guest wheels contain native
ARM64 AOT code; they are separate from the Linux/Windows x86-64 guest wheels.

The backend wheels include a signed Rust helper with the
`com.apple.security.hypervisor` entitlement. The SDK starts it automatically;
Python is not re-signed or modified. Guest execution uses one helper process per
active sandbox, with pooled reuse. This path is experimental, not for production.

## Snapshot Semantics

- `snapshot()` captures guest runtime state.
- Output file contents under `/output` persist across `run()` calls, but open
  file handles must be reopened in each run.
- `restore()` rewinds runtime state and clears output files.

## Build

```bash
just python-build
```
