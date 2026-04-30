# .NET SDK for hyperlight-sandbox

A .NET 8.0 SDK for running code in secure, sandboxed environments using [hyperlight](https://github.com/hyperlight-dev/hyperlight-sandbox). Execute Python, JavaScript, or custom guest code inside lightweight micro-VMs with tool dispatch, filesystem isolation, network allowlists, and snapshot/restore.

## Features

- **Secure code execution** — run untrusted code in an isolated sandbox
- **Tool dispatch** — register .NET functions callable from guest code via `call_tool()`
- **Typed tool registration** — auto-serialize/deserialize with `Func<TArgs, TResult>`
- **Two backends** — Wasm (Python/JS guests) and built-in JavaScript (QuickJS)
- **Snapshot/restore** — checkpoint and rewind sandbox state (200x faster warm starts)
- **Filesystem isolation** — read-only input dirs, writable output dirs, temp output
- **Network allowlists** — per-domain HTTP access control with method filtering
- **AI agent integration** — `CodeExecutionTool` with `AIFunction` for Copilot SDK and Microsoft Agent Framework
- **Thread-safe** — `lock`-based serialization allows safe cross-thread moves

## Quick Start

```bash
# Prerequisites
just wasm guest-build          # Build the Python guest module
just dotnet build              # Build the .NET SDK + Rust FFI
```

### Basic Usage

```csharp
using HyperlightSandbox.Api;

// Create a sandbox with the Python guest
using var sandbox = new SandboxBuilder()
    .WithModulePath("path/to/python-sandbox.aot")
    .Build();

// Execute code
var result = sandbox.Run("""
    import math
    primes = [n for n in range(2, 50)
              if all(n % i != 0 for i in range(2, int(math.sqrt(n)) + 1))]
    print(f"Primes: {primes}")
    """);

Console.WriteLine(result.Stdout);   // Primes: [2, 3, 5, 7, 11, ...]
Console.WriteLine(result.Success);  // True
```

### Tool Registration

Register .NET functions that guest code can call:

```csharp
using var sandbox = new SandboxBuilder()
    .WithModulePath("python-sandbox.aot")
    .Build();

// Typed tool — auto-serializes args and result
sandbox.RegisterTool<MathArgs, double>("add",
    args => args.A + args.B);

// Raw JSON tool
sandbox.RegisterTool("lookup", (string json) =>
    json.Contains("weather")
        ? """{"temp": 22, "condition": "sunny"}"""
        : """{"error": "unknown"}""");

var result = sandbox.Run("""
    sum = call_tool("add", a=10, b=32)
    print(f"10 + 32 = {sum}")

    weather = call_tool("lookup", key="weather")
    print(f"Weather: {weather}")
    """);

// DTO for typed tools
record MathArgs(double a, double b);
```

### JavaScript Backend

Use the built-in QuickJS runtime — no guest module needed:

```csharp
using var sandbox = new SandboxBuilder()
    .WithBackend(SandboxBackend.JavaScript)
    .Build();

var result = sandbox.Run("console.log('Hello from JS!');");
```

### Snapshot/Restore

Checkpoint sandbox state for fast resets between executions:

```csharp
using var sandbox = new SandboxBuilder()
    .WithModulePath("python-sandbox.aot")
    .Build();

// Cold start (~2.5s)
sandbox.Run("pass");

// Take snapshot of clean state
using var snapshot = sandbox.Snapshot();

// Execute code (modifies state)
sandbox.Run("x = 42");

// Restore to clean state (~2ms — 1000x faster than cold start)
sandbox.Restore(snapshot);
sandbox.Run("print(x)");  // NameError: x is not defined
```

### Filesystem Access

```csharp
using var sandbox = new SandboxBuilder()
    .WithModulePath("python-sandbox.aot")
    .WithInputDir("/path/to/input")    // Read-only /input in guest
    .WithTempOutput()                   // Writable /output in guest
    .Build();

sandbox.Run("""
    with open("/input/data.txt") as f:
        data = f.read()
    with open("/output/result.txt", "w") as f:
        f.write(data.upper())
    """);

var files = sandbox.GetOutputFiles();  // ["result.txt"]
var path = sandbox.OutputPath;         // /tmp/hyperlight-xxx/
```

### Network Allowlist

```csharp
sandbox.AllowDomain("https://httpbin.org");                    // All methods
sandbox.AllowDomain("https://api.example.com", ["GET", "POST"]); // Filtered

sandbox.Run("""
    response = http_get("https://httpbin.org/get")
    print(f"Status: {response['status']}")
    """);
```

### AI Agent Integration

Use with GitHub Copilot SDK or Microsoft Agent Framework:

```csharp
using HyperlightSandbox.Api;
using HyperlightSandbox.Extensions.AI;

// Create a code execution tool with snapshot/restore for clean state
using var codeTool = new CodeExecutionTool(
    new SandboxBuilder()
        .WithModulePath("python-sandbox.aot")
        .WithTempOutput());

codeTool.RegisterTool<MathArgs, double>("compute",
    args => args.A + args.B);

// Get as AIFunction for agent registration
var executeCode = codeTool.AsAIFunction();

// Use with Copilot SDK
var session = await client.CreateSessionAsync(new SessionConfig
{
    Tools = [executeCode],
});

// Use with Microsoft Agent Framework / IChatClient
var response = await chatClient.GetResponseAsync(prompt,
    new ChatOptions { Tools = [executeCode] });
```

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      .NET Application                    │
│                                                          │
│  ┌────────────────────┐  ┌─────────────────────────────┐ │
│  │ HyperlightSandbox  │  │ HyperlightSandbox           │ │
│  │        .Api        │  │      .Extensions.AI         │ │
│  │                    │  │                             │ │
│  │  Sandbox           │  │  CodeExecutionTool          │ │
│  │  SandboxBuilder    │  │    → AIFunction             │ │
│  │  ExecutionResult   │  │                             │ │
│  └─────────┬──────────┘  └──────────┬──────────────────┘ │
│            │                        │                    │
│  ┌─────────▼────────────────────────▼──────────────────┐ │
│  │          HyperlightSandbox.PInvoke                  │ │
│  │   SafeNativeMethods · SafeHandles · FFIResult       │ │
│  └──────────────────────┬──────────────────────────────┘ │
│                         │ P/Invoke                       │
└─────────────────────────┼────────────────────────────────┘
                          │
        ┌─────────────────▼───────────────────┐
        │   hyperlight_sandbox_ffi (cdylib)   │
        │   Rust FFI · Box::into_raw/from_raw │
        └─────────────────┬───────────────────┘
                          │
        ┌─────────────────▼───────────────────┐
        │   hyperlight-sandbox (Rust core)    │
        │   + hyperlight-wasm-sandbox         │
        │   + hyperlight-javascript-sandbox   │
        └─────────────────────────────────────┘
```

## Build Commands

```bash
just dotnet build              # Build Rust FFI + .NET solution
just dotnet test               # Run 93 xUnit tests
just dotnet test-rust          # Run 68 Rust FFI tests
just dotnet fmt-check          # Check .NET formatting
just dotnet fmt                # Apply .NET formatting
just dotnet analyze            # Roslyn analyzers (warnings as errors)
just dotnet examples           # Run core examples
just dotnet dist               # Build NuGet packages → dist/dotnetsdk/
just dotnet agent-framework-example   # Run MAF example
just dotnet copilot-sdk-example       # Run Copilot SDK example
```

## NuGet Packages

| Package | Description |
|---------|-------------|
| `Hyperlight.HyperlightSandbox.PInvoke` | P/Invoke bindings + native library |
| `Hyperlight.HyperlightSandbox.Api` | High-level API (Sandbox, tools, snapshots) |
| `Hyperlight.HyperlightSandbox.Extensions.AI` | AI agent integration (CodeExecutionTool, AIFunction) |

## API Reference

### `SandboxBuilder`

| Method | Description |
|--------|-------------|
| `WithModulePath(string)` | Path to `.wasm`/`.aot` guest (required for Wasm) |
| `WithBackend(SandboxBackend)` | `Wasm` (default) or `JavaScript` |
| `WithHeapSize(string\|ulong)` | Guest heap size (e.g. `"50Mi"`, default: platform-dependent) |
| `WithStackSize(string\|ulong)` | Guest stack size (e.g. `"35Mi"`, default: platform-dependent) |
| `WithInputDir(string)` | Read-only `/input` directory |
| `WithOutputDir(string)` | Writable `/output` directory |
| `WithTempOutput()` | Auto-created temp `/output` directory |
| `Build()` | Creates `Sandbox` instance |

### `Sandbox`

| Method | Description |
|--------|-------------|
| `Run(string code)` | Execute guest code, returns `ExecutionResult` |
| `RunAsync(string, CancellationToken)` | Async version on thread pool |
| `RegisterTool<TArgs,TResult>(name, handler)` | Register typed tool |
| `RegisterTool(name, Func<string,string>)` | Register raw JSON tool |
| `AllowDomain(target, methods?)` | Add domain to network allowlist |
| `GetOutputFiles()` | List files written to output |
| `OutputPath` | Host path of output directory |
| `Snapshot()` | Capture sandbox state |
| `Restore(snapshot)` | Restore to captured state |

### `ExecutionResult`

| Property | Type | Description |
|----------|------|-------------|
| `Stdout` | `string` | Captured standard output |
| `Stderr` | `string` | Captured standard error |
| `ExitCode` | `int` | Guest exit code (0 = success) |
| `Success` | `bool` | `true` if `ExitCode == 0` |

### Exceptions

| Exception | When |
|-----------|------|
| `SandboxException` | Base type for all sandbox errors |
| `SandboxTimeoutException` | Execution exceeded time limit |
| `SandboxPoisonedException` | Sandbox state corrupted (recreate) |
| `SandboxPermissionException` | Network access denied |
| `SandboxGuestException` | Guest code raised an error |

## Thread Safety

The `Sandbox` class is **Send but not Sync** — it can be moved between threads but concurrent access is serialized via an internal lock. For parallel execution, create one sandbox per thread.

```csharp
// ✅ OK — move between threads via Task.Run
var result = await sandbox.RunAsync("print('hello')");

// ✅ OK — sequential access from different threads
await Task.Run(() => sandbox.AllowDomain("https://example.com"));
sandbox.Run("...");

// ⚠️ Serialized — concurrent calls block, don't deadlock
// For throughput, use one sandbox per thread
```

## Requirements

- .NET 8.0 SDK or later
- Rust 1.89+ (for building the FFI crate)
- Linux (Windows support coming via hyperlight)
- `just wasm guest-build` for Wasm backend examples

## Contributing

When adding new FFI functions:

1. Add `extern "C"` export in `src/sdk/dotnet/ffi/src/lib.rs`
2. Add `[LibraryImport]` declaration in `PInvoke/SafeNativeMethods.cs`
3. Wrap in high-level API in `Api/`
4. Add tests
5. Run `just dotnet fmt` and `just dotnet analyze`
6. Ensure all tests pass with `just dotnet test`
