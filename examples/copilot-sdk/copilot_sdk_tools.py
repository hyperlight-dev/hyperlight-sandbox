#!/usr/bin/env python3
"""GitHub Copilot SDK + Hyperlight Wasm Sandbox integration example.

This demonstrates using the GitHub Copilot SDK (https://github.com/github/copilot-sdk)
with three registered SDK tools: execute_code, compute, and fetch_data.

Architecture:
    ┌────────────────────────────────────────────────────────────────────────────┐
    │  GitHub Copilot SDK                                                        │
    │    async with CopilotClient() as client:                                   │
    │        async with await client.create_session({                            │
    │            "model": "gpt-5",                                             │
    │            "tools": get_tools(),                                          │
    │            "system_message": {...},                                       │
    │        }) as session:                                                      │
    │            session.on(on_event)                                            │
    │           │                                                                │
    │           ▼                                                                │
    │    Model sees schemas for execute_code, compute, and fetch_data            │
    │    Prompt steers it to call execute_code and use call_tool() in sandbox    │
    └────────────────────────────────────────────────────────────────────────────┘
                 │
                 ▼
    ┌────────────────────────────────────────────────────────────────────────────┐
    │  Sandbox(backend="wasm", ...)                                             │
    │    sandbox.register_tool(tool) for SDK Tool objects                        │
    │    (compute/fetch_data adapted to guest-callable callbacks)                │
    │    sandbox.run(code=...)                                                   │
    │           │                                                                │
    │           ▼                                                                │
    │    Guest: call_tool("fetch_data", table="users")                          │
    │    Guest: call_tool("compute", operation="multiply", a=6, b=7)            │
    │           │                                                                │
    │           ▼                                                                │
    │    Host callbacks execute and return results back to guest code            │
    └────────────────────────────────────────────────────────────────────────────┘

Key Concepts:
    1. execute_code - the primary SDK tool for isolated execution in Hyperlight
    2. compute/fetch_data - SDK-visible host tools whose schemas guide the model
    3. Guest-to-host callbacks - sandboxed code calls host functions via call_tool()
       (injected as a built-in; `from hyperlight import call_tool` also works)
    4. Session events - tool.executionStart events log tool usage and flag direct compute/fetch_data calls

This enables powerful patterns where sandboxed code can access controlled host
capabilities (database queries, API calls, etc.) while remaining isolated.

Prerequisites:
    uv sync --group copilot-sdk
    gh auth login

    # Build the AOT module:
    # Default: src/wasm_sandbox/guests/python/python-sandbox.aot

Usage:
    # From repo root
    python examples/copilot-sdk/copilot_sdk_tools.py

Compare with:
    - ../../src/sdk/python/core/examples/basic.py - Basic Sandbox(backend="wasm", ...) usage
"""

import argparse
import asyncio
from pathlib import Path
from typing import Any

from pydantic import BaseModel, Field

EXECUTE_CODE_DESCRIPTION = (
    "Execute Python code securely in an isolated Hyperlight Wasm sandbox. "
    "Inside the sandbox, call_tool() is available as a built-in — no import needed. "
    "Use call_tool('compute', operation=..., a=..., b=...) and "
    "call_tool('fetch_data', table=...) to invoke host tools."
)
COMPUTE_DESCRIPTION = (
    "Perform a math operation on two numbers. This tool is registered so the model "
    "can see its schema, but it should normally be called from sandboxed code via "
    "execute_code and call_tool()."
)
FETCH_DATA_DESCRIPTION = (
    "Fetch simulated records from a named table. This tool is registered so the model "
    "can see its schema, but it should normally be called from sandboxed code via "
    "execute_code and call_tool()."
)

COPILOT_SYSTEM_PROMPT = """You have tools available. Discover them but don't call them directly.
IMPORTANT: Do NOT call anything other than execute_code directly. Instead, write
Python code using execute_code that calls them via call_tool() inside the sandbox.

You do NOT have direct access to any data. The ONLY way to fetch data or perform
computations is by calling call_tool() inside sandbox code. NEVER hardcode data.

call_tool is a built-in global — no import needed.

You MUST use call_tool() for:
  - call_tool('fetch_data', table='users')   # fetches live data
  - call_tool('compute', operation='multiply', a=6, b=7)  # performs math

NEVER hardcode data that should come from call_tool:
  users = [{"name": "Alice"}]  # WRONG — always use call_tool('fetch_data', ...)

Prefer solving each user request in a single execute_code call when possible."""

DEFAULT_PROMPT = (
    "Fetch all users, find admins, multiply 6*7, and print the users, "
    "admins, and multiplication result. Use one execute_code call."
)


class ExecuteCodeParams(BaseModel):
    """Parameters for the execute_code tool."""

    code: str = Field(
        description="Python code to execute in isolated Hyperlight Wasm sandbox. Use call_tool('fetch_data', table=...) and call_tool('compute', operation=..., a=..., b=...) inside the code to access data and perform calculations. NEVER hardcode data."
    )


class ComputeParams(BaseModel):
    """Parameters for the compute tool."""

    operation: str = Field(description="Math operation to perform: add, subtract, multiply, or divide")
    a: float = Field(description="First numeric operand")
    b: float = Field(description="Second numeric operand")


class FetchDataParams(BaseModel):
    """Parameters for the fetch_data tool."""

    table: str = Field(description="Name of the simulated table to query, such as users or products")


# --- Simulated Data ---

_SIMULATED_DATA = {
    "users": [
        {"id": 1, "name": "Alice", "role": "admin"},
        {"id": 2, "name": "Bob", "role": "user"},
        {"id": 3, "name": "Charlie", "role": "admin"},
    ],
    "products": [
        {"id": 101, "name": "Widget", "price": 9.99},
        {"id": 102, "name": "Gadget", "price": 19.99},
    ],
}


# --- Tool Definitions (single source of truth) ---
# get_tools() defines the Copilot SDK Tool objects used by both the SDK session and sandbox.


def compute(params: ComputeParams) -> float:
    """Math operations: add, subtract, multiply, divide."""
    match params.operation:
        case "add":
            return params.a + params.b
        case "subtract":
            return params.a - params.b
        case "multiply":
            return params.a * params.b
        case "divide":
            return params.a / params.b if params.b != 0 else float("inf")
        case _:
            return 0.0


async def fetch_data(params: FetchDataParams) -> list[dict[str, Any]]:
    """Query a simulated database table (async — e.g. could hit a real DB)."""
    await asyncio.sleep(0)  # simulate async I/O
    return _SIMULATED_DATA.get(params.table, [])


# --- Sandbox singleton with snapshot/restore ---
# The sandbox is created once at startup (cold start ~680ms), snapshotted, then
# restored before each execute_code call for clean state with fast startup.

_sandbox = None
_snapshot = None


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def _default_module_path() -> Path:
    return _repo_root() / "src/wasm_sandbox/guests/python/python-sandbox.aot"


def _init_sandbox(module_path: str | None = None):
    """Initialize the sandbox and take a snapshot. Call once at program start."""
    global _sandbox, _snapshot
    import time as _time

    from hyperlight_sandbox import Sandbox

    resolved_module_path = Path(module_path) if module_path else _default_module_path()

    if not resolved_module_path.exists():
        raise RuntimeError(
            f"Hyperlight Wasm module not found.\n"
            f"  module: {resolved_module_path} (MISSING)\n"
            "Build the python-sandbox AOT module first, or pass --module-path."
        )

    start = _time.perf_counter()
    _sandbox = Sandbox(backend="wasm", module_path=str(resolved_module_path))

    # Register tool functions as host callbacks.
    # Sync and async functions both work — async is awaited automatically.
    _sandbox.register_tool("compute", lambda **kw: compute(ComputeParams(**kw)))

    async def _fetch_data_cb(**kw):
        return await fetch_data(FetchDataParams(**kw))

    _sandbox.register_tool("fetch_data", _fetch_data_cb)

    # Warm up the sandbox (first run triggers init) and snapshot clean state
    _sandbox.run("None")
    _snapshot = _sandbox.snapshot()
    elapsed_ms = (_time.perf_counter() - start) * 1000
    print(f"\U0001f4f8 Sandbox initialized and snapshotted ({elapsed_ms:.0f}ms)")


def _get_sandbox():
    """Restore sandbox to clean snapshot state and return it."""
    _sandbox.restore(_snapshot)
    return _sandbox


async def execute_code(params: ExecuteCodeParams) -> str:
    """Execute Python code in an isolated Hyperlight Wasm sandbox.

    The sandbox has call_tool() available as a built-in global:
        result = call_tool("tool_name", key=value, ...)

    Uses snapshot/restore to reset state between calls.

    Returns:
        Execution output (stdout) or error message.
    """
    import time

    try:
        print(f"--- Copilot generated code ---\n{params.code}\n--- end ---\n")
        sandbox = _get_sandbox()

        start = time.perf_counter()
        result = sandbox.run(code=params.code)
        elapsed_ms = (time.perf_counter() - start) * 1000

        if result.exit_code == 0:
            stdout = result.stdout
            print(f"⏱️  execute_code completed ({elapsed_ms:.1f}ms)")
            return stdout if stdout else "Code executed successfully (no output)."

        stderr = result.stderr or "Unknown error"
        print(f"⏱️  execute_code failed ({elapsed_ms:.1f}ms)")
        return f"Execution error:\n{stderr}"
    except Exception as exc:
        return f"Sandbox error: {exc}"


def get_tools() -> list[Any]:
    """Wrap the plain tool functions with define_tool() for the Copilot SDK."""
    from copilot import define_tool

    return [
        define_tool(
            name="execute_code",
            description=EXECUTE_CODE_DESCRIPTION,
            handler=execute_code,
            params_type=ExecuteCodeParams,
        ),
        define_tool(
            name="compute",
            description=COMPUTE_DESCRIPTION,
            handler=compute,
            params_type=ComputeParams,
        ),
        define_tool(
            name="fetch_data",
            description=FETCH_DATA_DESCRIPTION,
            handler=fetch_data,
            params_type=FetchDataParams,
        ),
    ]


# --- Full Copilot SDK Example ---


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the Copilot SDK sandbox demo")
    parser.add_argument(
        "--prompt",
        default=DEFAULT_PROMPT,
        help="Prompt to send to the Copilot SDK session",
    )
    parser.add_argument(
        "--module-path",
        help="Path to the Hyperlight Wasm AOT module. Defaults to the repo guest build output.",
    )
    return parser.parse_args()


async def run_copilot_demo(prompt: str, module_path: str | None = None):
    """Run a complete demo using the GitHub Copilot SDK."""
    try:
        from copilot import CopilotClient, PermissionHandler
    except ImportError:
        print("Error: copilot SDK not installed")
        print("Install with: uv sync --group copilot-sdk")
        return

    print("=== GitHub Copilot SDK + Hyperlight Wasm Demo ===\n")

    _init_sandbox(module_path)  # pay cold start once, upfront

    client = CopilotClient()
    await client.start()

    try:
        session = await client.create_session(
            {
                "model": "gpt-5",
                "tools": get_tools(),
                "system_message": {"content": COPILOT_SYSTEM_PROMPT},
                "on_permission_request": PermissionHandler.approve_all,
            }
        )

        # Log SDK-level tool calls as they happen
        tool_calls: dict[str, str] = {}  # tool_call_id → tool_name

        def on_event(event):
            etype = str(getattr(event, "type", ""))
            if "TOOL" not in etype.upper():
                return
            data = getattr(event, "data", None)
            tool_name = getattr(data, "tool_name", None) or getattr(data, "name", None)
            tool_call_id = getattr(data, "tool_call_id", None)
            is_sandbox = tool_call_id == "sandbox"
            origin = "sandbox → host" if is_sandbox else "copilot → sdk"
            if "START" in etype.upper():
                if tool_call_id and tool_name:
                    tool_calls[tool_call_id] = tool_name
                icon = "🔗" if is_sandbox else "🔧"
                print(f"{icon} [{origin}] {tool_name or tool_call_id}")
            elif "COMPLETE" in etype.upper():
                if not tool_call_id:
                    return
                name = tool_calls.get(tool_call_id, tool_call_id)
                success = getattr(data, "success", True)
                duration = getattr(data, "duration", None)
                status = "✅" if success else "❌"
                timing = f" ({duration}ms)" if duration else ""
                print(f"{status} [{origin}] {name} done{timing}")

        session.on(on_event)

        print(f"User: {prompt}\n")

        response = None
        max_retries = 3
        for attempt in range(1, max_retries + 1):
            try:
                timeout_secs = 60.0 * attempt  # 60s, 120s, 180s
                response = await session.send_and_wait({"prompt": prompt}, timeout=timeout_secs)
                break
            except TimeoutError:
                if attempt < max_retries:
                    print(f"⏱️  Attempt {attempt}/{max_retries} timed out after {timeout_secs}s, retrying...")
                else:
                    print(f"⏱️  All {max_retries} attempts timed out")
                    raise

        if response and hasattr(response, "data") and hasattr(response.data, "content"):
            print(f"\nCopilot: {response.data.content}")
        else:
            print(f"\nCopilot: {response}")

        await session.disconnect()
    finally:
        await client.stop()


if __name__ == "__main__":
    args = parse_args()
    asyncio.run(run_copilot_demo(args.prompt, args.module_path))
