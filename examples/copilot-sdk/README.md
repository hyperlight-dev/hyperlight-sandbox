# GitHub Copilot SDK + Hyperlight Wasm Sandbox

Run sandboxed Python code in Hyperlight Wasm sandboxes, orchestrated by GitHub Copilot.

## Quick Start

```bash
# From repo root
uv sync --group copilot-sdk

# Build the guest AOT module
just guest-build

# Build/install the local Hyperlight Python package
just python-build

gh auth login

just copilot-sdk-example
```

## How It Works

Copilot sees three tools (`execute_code`, `compute`, `fetch_data`) but the system prompt steers it to write Python code via `execute_code`. Inside the Wasm sandbox, the generated code calls host functions through `call_tool()` (a built-in global).

```
Copilot → execute_code(code="...")
              │
              ▼
         Sandbox(backend="wasm", ...).run(code)
              │
              ├── call_tool("fetch_data", table="users")  → host
              ├── call_tool("compute", op="multiply", a=6, b=7)  → host
              │
              ▼
         stdout returned to Copilot
```

### Example Output

```
🔧 [copilot → sdk] execute_code
--- Copilot generated code ---
users = call_tool("fetch_data", table="users")
result = call_tool("compute", operation="multiply", a=6, b=7)
admins = [u for u in users if u.get("role") == "admin"]
print(f"Admins: {[a['name'] for a in admins]}")
print(f"6 * 7 = {result}")
--- end ---

✅ [copilot → sdk] execute_code done
```

