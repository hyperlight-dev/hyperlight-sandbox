---
marp: true
html: true
theme: default
paginate: true
title: Hyperlight Sandbox
description: End-user overview of the Python SDK, capabilities, backend timings, and the agent framework scenario.
style: |
  section {
    padding-top: 30px;
  }
  section h1 {
    margin-bottom: 0.3em;
  }
  section h2 {
    margin-top: 0.2em;
    margin-bottom: 0.2em;
  }
  pre {
    margin-top: 0.3em;
    margin-bottom: 0.3em;
    background: #1e1e2e;
  }
  code {
    color: #cdd6f4;
  }
  :not(pre) > code {
    color: #1e1e2e;
    background: #e0e0e0;
  }
  pre code .hljs-comment { color: #7f849c; }
  pre code .hljs-string { color: #a6e3a1; }
  pre code .hljs-keyword { color: #cba6f7; }
  pre code .hljs-built_in { color: #89b4fa; }
  pre code .hljs-function { color: #89b4fa; }
  pre code .hljs-title { color: #89b4fa; }
  pre code .hljs-number { color: #fab387; }
  pre code .hljs-params { color: #f5e0dc; }
  pre code .hljs-attr { color: #89dceb; }
  .green { color: #2ea043; font-weight: bold; }
  .red { color: #f85149; font-weight: bold; }
  section.timing { font-size: 22px; }
  section.timing table { font-size: 20px; }
---

# Hyperlight Sandbox

Fast sandboxed execution for agents, tools, and apps

- Run untrusted code in an isolated sandbox
- Keep a simple Python API
- Add tools, files, snapshots, and network rules when you need them

---

# Hello World

```python
from hyperlight_sandbox import Sandbox

sandbox = Sandbox(backend="wasm", module="python_guest.path")
result = sandbox.run('print("Hello, World!")')
print(result.stdout)
```

That's it - create a sandbox, run code, read the output.

---

# Use Cases

- **File Processing**: Process provided files in Python and return a summarized report
- **Code Mode**: Let an agent write a script that calls your tools directly, reducing token usage
- **Sandboxed Execution** as a library: drop into an existing app or library without building a custom runtime
- **Agent Skills** combine scripts into multi-step workflows that run in isolation (future)


---

# Where It Fits Best

- Embedded code execution in apps
- Safe plugin or user-script scenarios
- Data workflows that need controlled host access
- Agent workflows that need repeatable, isolated execution

---

# Guest Support: SDK Backends

## Python guest in Wasm (componentize-py)

```python
sandbox = Sandbox(backend="wasm", module="python_guest.path")
result = sandbox.run('print(2 + 3)')
```

## JavaScript guest in Wasm (componentize-js)

```python
sandbox = Sandbox(backend="wasm", module="javascript_guest.path")
result = sandbox.run('console.log(2 + 3)')
```

## HyperlightJS backend (quickjs)

```python
sandbox = Sandbox(backend="hyperlight-js")
result = sandbox.run('console.log(2 + 3)')
```

---

# Guest Support: Nanvix & Extensibility

## Nanvix guests (Rust API)

```rust
// JavaScript via QuickJS in a Nanvix microkernel
let mut sandbox = SandboxBuilder::new().guest(NanvixJavaScript).build()?;
let result = sandbox.run(r#"console.log("Hello from Nanvix!")"#)?;

// Python via Python in a Nanvix microkernel
let mut sandbox = SandboxBuilder::new().guest(NanvixPython).build()?;
let result = sandbox.run(r#"print("Hello from Nanvix!")"#)?;
```

## Broader Wasm/WASI guest model

- The Wasm path is not limited to Python and JavaScript
- Any runtime or language that can target the expected Wasm/WASI guest model can fit here
- Python and JavaScript are just the packaged end-user examples in this repo today

---

# Capabilities

Capability based external access. Off by default.

```python
import tempfile
from pathlib import Path

input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
Path(input_dir, "data.json").write_text('{"name": "Alice"}')

sandbox = Sandbox(backend="wasm", module="python_guest.path", input_dir=input_dir)
sandbox.allow_domain("https://example.com", methods=["GET"])
```

- Files: read from `/input`, write to `/output`
- Network: outbound HTTP with a domain allowlist

---

# Snapshots

Capture state, restore it later. Pay cold start once, reuse across mutliple tenants.

```python
sandbox = Sandbox(backend="wasm", module="python_guest.path")
sandbox.register_tool("fetch_data", lambda table="": db.query(table))

# Warm up the runtime once
sandbox.run("")
snap = sandbox.snapshot()

# Each task starts from a clean, warm state
for task in tasks:
    sandbox.restore(snap)
    result = sandbox.run(task)
    print(result.stdout)
```

- `snapshot()` captures runtime state and files
- `restore()` rewinds to that point — fast, no cold start
- Tools survive a restore; runtime and file state do not

---

# Capabilities: Tools

Register host functions and call them from sandboxed code.

```python
sandbox = Sandbox(backend="wasm", module="python_guest.path")
sandbox.register_tool("fetch_data", lambda table="": db.query(table))

result = sandbox.run("""
users = call_tool('fetch_data', table='users')
sales = call_tool('fetch_data', table='sales')
print(f"Found {len(users)} users and {len(sales)} sales")
""")
```

- Works for sync and async functions
- Lets sandboxed code ask the host for data or computation safely

---

# Capabilities: Files

```python
import json, os, tempfile
from pathlib import Path

input_dir = tempfile.mkdtemp(prefix="sandbox-input-")
Path(input_dir, "data.json").write_text('{"name": "Alice"}')

sandbox = Sandbox(
  backend="wasm",
  module="python_guest.path",
  input_dir=input_dir,
  temp_output=True,
)
result = sandbox.run("""
import json, os

# Host-provided file is available
with open('/input/data.json') as f:
    data = json.load(f)

# Files not added by the host don't exist
print(os.path.exists('/input/secret.env'))  # False

with open('/output/report.txt', 'w') as f:
    f.write(f"Hello, {data['name']}")
""")
output_dir = sandbox.output_path()
with open(os.path.join(output_dir, "report.txt")) as f:
  report = f.read()
```

---

# Capabilities: Network

```python
# Allow GET to example.com
sandbox.allow_domain("https://example.com", methods=["GET"])

# 🟢 Allowed — GET to example.com
resp = http_get("https://example.com/api/users")

# 🟥 Denied — POST to example.com, wrong method
resp = http_post("https://example.com/api/users", ...)

# 🟥 Denied — GET to other.com, not in allowlist
resp = http_get("https://other.com/admin")
```

- Deny-by-default — no network access until you opt in
- Allowlist by domain and HTTP method
- `allow_domain("https://domain")` permits all methods; `methods=["GET"]` restricts further

---

<!-- _class: timing -->

# Timing Notes

Measured from `benchmark.py` (release build) — 5 cold / 10 warm rounds, averages shown.

==========================================================================================
Step                                    Wasm + Python Wasm + JavaScript      HyperlightJS
-----------------------------------------------------------------------------------------
Cold start (create + first run)              143.9 ms          119.9 ms           96.7 ms
Warm run (no restore)                          0.2 ms            0.3 ms            0.1 ms
Cold start + tool dispatch                   138.1 ms          114.5 ms          105.4 ms
Warm tool dispatch (no restore)                0.4 ms            0.3 ms            0.1 ms
Cold start + file I/O                        125.7 ms          119.4 ms          101.8 ms
Warm file I/O (no restore)                     0.3 ms            0.4 ms            0.1 ms
Snapshot                                      59.1 ms           59.7 ms           20.5 ms
Restore                                       11.4 ms           11.1 ms           11.5 ms
Restore + run                                  1.5 ms            1.3 ms            0.7 ms
Restore + tool dispatch                        1.3 ms            1.2 ms            0.6 ms


---

# What Snapshot Buys You

Instead of paying cold start every time:

1. Create the sandbox once
2. Warm it up once
3. Take a clean snapshot
4. Restore before each new task to avoid sharing information

That gives you:

- Clean state between runs
- Fast repeat execution
- Predictable behavior for agents and multi-step workflows

---

# Agent Framework Scenario

The agent example uses Hyperlight Sandbox as the safe execution engine.

```text
Copilot Agent
    |
    |  execute_code(code="...")
    v
+-------------------------------------------+
|  Sandbox                                  |
|                                           |
|    sandbox.run(code)                      |
|        |                                  |
|        |-- call_tool("fetch_data") -------+--> Host
|        |-- call_tool("compute")    -------+--> Host
|        |                                  |
|        v                                  |
|    stdout captured                        |
+-------------------------------------------+
    |
    v
result returned to agent
```
