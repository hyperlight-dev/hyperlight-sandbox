# GitHub Copilot Agent + Hyperlight Wasm Sandbox

Run sandboxed Python code in Hyperlight Wasm sandboxes with Microsoft Agent Framework and GitHub Copilot.

## Quick Start

```bash
# From repo root
uv sync --group agent-framework

gh auth login

# Build the guest AOT module
just guest-build

# Build/install the local Hyperlight Python package
just python-build

just agent-framework-example

# Interactive multi-turn REPL
just agent-framework-example-interactive

# DevUI web interface
uv sync --group agent-framework-devui
just agent-framework-example-devui
```

## How It Works

Copilot sees three tools (`execute_code`, `compute`, `fetch_data`) but the system prompt steers it to write Python code via `execute_code`. Inside the Wasm sandbox, generated code calls host functions through `call_tool()` (a built-in global).

```
Copilot Agent → execute_code(code="...")
                  │
                  ▼
             Sandbox(backend="wasm", ...).run(code)
                  │
                  ├── call_tool("fetch_data", table="users") → host
                  ├── call_tool("compute", operation="multiply", a=6, b=7) → host
                  ▼
             stdout returned to the agent
```

`compute` and `fetch_data` are exposed to the model for schema guidance, and `_create_sandbox()` registers them as host callbacks so sandboxed code can call them via `call_tool()`.
