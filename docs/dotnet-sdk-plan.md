# .NET SDK Implementation Plan — hyperlight-sandbox

> **Status**: 🟡 In Progress
> **Last Updated**: 2026-04-13
> **Tracking**: Update this document as phases complete.

## Overview

Create an idiomatic .NET 8.0+ SDK for hyperlight-sandbox, mirroring the Python SDK's API surface (`Sandbox`, `ExecutionResult`, tool registration, filesystem, networking, snapshots). Architecture follows [PR deislabs/hyperlight-js#292](https://github.com/deislabs/hyperlight-js/pull/292) (Rust cdylib → P/Invoke → high-level C# API).

**Key deliverables:**
- Core SDK: `HyperlightSandbox.Api` + `HyperlightSandbox.PInvoke` NuGet packages
- Extensions: `HyperlightSandbox.Extensions.AI` for agent framework integration
- Samples: GitHub Copilot SDK + Microsoft Agent Framework examples
- Tests: 93+ xUnit tests across 9 test classes
- CI: Integrated into existing GitHub Actions workflows

## Decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| 1 | Backend | WASM only initially | nanvix requires nightly; hyperlight-js can be added later |
| 2 | Target Framework | net8.0 (consumers on 9.0+ just use it) | Current LTS |
| 3 | FFI approach | Rust cdylib + P/Invoke | Proven pattern from PR #292 |
| 4 | Tool callbacks | Function pointer (`[UnmanagedCallersOnly]`) | Synchronous dispatch matching Python SDK |
| 5 | Async tools | DEFERRED — sync-only initially | Async risks deadlocks on sandbox thread |
| 6 | Async pattern | Sync primary, `Task.Run()` async convenience | Per PR #292 guidance |
| 7 | JSON | `System.Text.Json` only | No Newtonsoft; matches PR #292 |
| 8 | Thread safety | Not thread-safe; thread-affinity check | Document clearly |
| 9 | Module resolution | Explicit path to `.wasm`/`.aot` only (BYOM) | Simplified vs Python's importlib resolution |
| 10 | Guest modules | Users bring their own | No pre-built guest NuGet packages |
| 11 | Error handling | Both FFI error codes AND custom C# exceptions | Structured classification across boundary |
| 12 | Extensions.AI | Included in initial scope | Agent framework samples need it |
| 13 | Location | `src/sdk/dotnet/` | Parallel to `src/sdk/python/` |

## Issues from PR #292 to Address

| PR Issue | .NET SDK Impact | Status |
|----------|----------------|--------|
| #440 — Thread safety | Thread-affinity check (capture `ManagedThreadId` in ctor, assert) | ⬜ |
| #439 — Host function registration | Full implementation via function pointer callbacks | ⬜ |
| #437 — Tracing | Defer; add `Activity` span extension points later | ⬜ Deferred |
| #438 — Logging | Defer; errors via FFIResult for now | ⬜ Deferred |
| #442 — Metrics | Defer | ⬜ Deferred |
| #443 — Unified FFI layer | Design FFI with future JS backend in mind | ⬜ |
| GC.KeepAlive barriers | After every FFI call using SafeHandle | ⬜ Critical |
| GCHandle pinning | Tool delegates pinned to prevent GC while Rust holds fn ptr | ⬜ Critical |

---

## Phase 1: FFI Foundation (Rust cdylib) ⚙️

> **Status**: ✅ Complete (68 tests passing, 0 warnings)
> **Depends on**: Nothing
> **Crate**: `src/sdk/dotnet/ffi/`

### Steps

- [ ] **1.1** Create `src/sdk/dotnet/ffi/Cargo.toml`
  - Depends on `hyperlight-sandbox` (core traits), `hyperlight-wasm-sandbox` (Wasm backend), `serde_json`, `log`
  - `crate-type = ["cdylib"]`

- [ ] **1.2** Create `src/sdk/dotnet/ffi/src/lib.rs` — C-compatible FFI exports (~800-1000 LOC)

  **Types:**
  - `FFIResult { is_success: bool, error_code: u32, value: *mut c_char }` — extended from PR #292 with error codes
  - `FFIErrorCode` enum: `Success = 0`, `Unknown = 1`, `Timeout = 2`, `Poisoned = 3`, `PermissionDenied = 4`, `GuestError = 5`, `InvalidArgument = 6`, `IoError = 7`
  - `FFISandboxOptions { module_path, heap_size, stack_size, ... }` — configuration struct
  - `ToolCallbackFn = extern "C" fn(args_json: *const c_char) -> *mut c_char` — tool callback type

  **Sandbox lifecycle:**
  - `hyperlight_sandbox_create(options: FFISandboxOptions, out handle) -> FFIResult`
  - `hyperlight_sandbox_free(handle)`

  **Configuration (pre-run):**
  - `hyperlight_sandbox_set_input_dir(handle, path) -> FFIResult`
  - `hyperlight_sandbox_set_output_dir(handle, path) -> FFIResult`
  - `hyperlight_sandbox_set_temp_output(handle, enabled) -> FFIResult`
  - `hyperlight_sandbox_allow_domain(handle, target, methods_json) -> FFIResult`

  **Tool registration:**
  - `hyperlight_sandbox_register_tool(handle, name, schema_json, callback) -> FFIResult`
  - Schema JSON: `{"args": {"a": "Number", "b": "Number"}, "required": ["a", "b"]}`

  **Execution:**
  - `hyperlight_sandbox_run(handle, code) -> FFIResult` (value = JSON `{"stdout":"...","stderr":"...","exit_code":0}`)

  **Filesystem:**
  - `hyperlight_sandbox_get_output_files(handle) -> FFIResult` (value = JSON array)
  - `hyperlight_sandbox_output_path(handle) -> FFIResult` (value = path string or null)

  **Snapshot/Restore:**
  - `hyperlight_sandbox_snapshot(handle) -> FFIResult` (value = snapshot handle)
  - `hyperlight_sandbox_restore(handle, snapshot) -> FFIResult`
  - `hyperlight_sandbox_free_snapshot(snapshot)`

  **Utility:**
  - `hyperlight_sandbox_free_string(ptr)`
  - `hyperlight_sandbox_get_version() -> *mut c_char`

  **Patterns** (from PR #292):
  - `safe_cstring()` helper for null-byte sanitization
  - Null-pointer checks on all inputs
  - `Box::into_raw` / `Box::from_raw` for handle management
  - All errors as UTF-8 `CString` pointers, caller frees

- [ ] **1.3** Add `src/sdk/dotnet/ffi` to root `Cargo.toml` workspace members

### Reference files
- `src/hyperlight_sandbox/src/lib.rs` — `SandboxBuilder`, `Sandbox<G>`, `Guest` trait
- `src/hyperlight_sandbox/src/tools.rs` — `ToolRegistry`, `ToolSchema`, `ArgType`
- `src/sdk/python/wasm_backend/src/lib.rs` — Lazy init, tool registry building

---

## Phase 2: P/Invoke Layer (.NET) 🔌

> **Status**: ✅ Complete (0 warnings, 0 errors, format clean, analyzers clean)
> **Depends on**: Phase 1
> **Project**: `src/sdk/dotnet/core/PInvoke/HyperlightSandbox.PInvoke.csproj`

### Steps

- [ ] **2.1** Create `HyperlightSandbox.PInvoke.csproj`
  - `TargetFramework: net8.0`, `AllowUnsafeBlocks`, `Nullable`, `AnalysisMode: All`
  - NuGet metadata: `Hyperlight.HyperlightSandbox.PInvoke`

- [ ] **2.2** Create `AssemblyInfo.cs` — `[assembly: DisableRuntimeMarshalling]`

- [ ] **2.3** Create `FFIResult.cs`
  - `[StructLayout(LayoutKind.Sequential)]` struct matching Rust `FFIResult`
  - `ThrowIfError()` — maps `error_code` to exception types
  - `StringFromPtr(IntPtr)` — reads + frees string

- [ ] **2.4** Create `FFIErrorCode.cs` — enum matching Rust `FFIErrorCode`

- [ ] **2.5** Create `SafeHandles.cs`
  - `SandboxSafeHandle` → calls `hyperlight_sandbox_free`
  - `SnapshotSafeHandle` → calls `hyperlight_sandbox_free_snapshot`
  - Each with `MakeHandleInvalid()`, `Interlocked.Exchange` in `ReleaseHandle()`

- [ ] **2.6** Create `SafeNativeMethods.cs`
  - All `[LibraryImport]` declarations
  - `[UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]`
  - `[MarshalAs(UnmanagedType.LPUTF8Str)]` for strings
  - Custom `NativeLibrary.SetDllImportResolver` for platform-specific loading
  - `#pragma warning disable CA5392` with justification

- [ ] **2.7** Create `SandboxOptions.cs` — `FFISandboxOptions` struct

- [ ] **2.8** Create `ToolCallbackDelegate.cs` — `[UnmanagedFunctionPointer(CallingConvention.Cdecl)]`

- [ ] **2.9** Create `build/net8.0/Hyperlight.HyperlightSandbox.PInvoke.targets` — native lib copy for NuGet consumers

### Reference files
- PR #292: `src/dotnet-host-api/src/dotnet/PInvoke/` — all files

---

## Phase 3: High-Level C# API 🎯

> **Status**: ✅ Complete (0 warnings, 0 errors, format clean, thread-affinity + GCHandle pinning)
> **Depends on**: Phase 2
> **Project**: `src/sdk/dotnet/core/Api/HyperlightSandbox.Api.csproj`

### Steps

- [ ] **3.1** Create `HyperlightSandbox.Api.csproj`
  - References `HyperlightSandbox.PInvoke`
  - NuGet metadata: `Hyperlight.HyperlightSandbox.Api`

- [ ] **3.2** Create `ExecutionResult.cs`
  ```csharp
  public sealed record ExecutionResult(string Stdout, string Stderr, int ExitCode)
  {
      public bool Success => ExitCode == 0;
  }
  ```

- [ ] **3.3** Create `SandboxBuilder.cs` — fluent builder
  ```csharp
  public class SandboxBuilder
  {
      public SandboxBuilder WithModulePath(string path);
      public SandboxBuilder WithHeapSize(string size);     // "25Mi" format
      public SandboxBuilder WithStackSize(string size);
      public SandboxBuilder WithInputDir(string path);
      public SandboxBuilder WithOutputDir(string path);
      public SandboxBuilder WithTempOutput(bool enabled = true);
      public Sandbox Build();
  }
  ```

- [ ] **3.4** Create `Sandbox.cs` — main API class
  ```csharp
  public sealed class Sandbox : IDisposable
  {
      // Tool registration (must be called before first Run)
      public void RegisterTool(string name, Delegate handler);
      public void RegisterTool<TArgs, TResult>(string name, Func<TArgs, TResult> handler);

      // Code execution
      public ExecutionResult Run(string code);
      public Task<ExecutionResult> RunAsync(string code);

      // Network
      public void AllowDomain(string target, IReadOnlyList<string>? methods = null);

      // Filesystem
      public IReadOnlyList<string> GetOutputFiles();
      public string? OutputPath { get; }

      // Snapshot/Restore
      public SandboxSnapshot Snapshot();
      public Task<SandboxSnapshot> SnapshotAsync();
      public void Restore(SandboxSnapshot snapshot);
      public Task RestoreAsync(SandboxSnapshot snapshot);

      public const int MaxCodeSize = 10 * 1024 * 1024;

      public void Dispose();
  }
  ```

  **Implementation details:**
  - Thread-affinity check: capture `Thread.CurrentThread.ManagedThreadId` in ctor, assert on each public method
  - `GC.KeepAlive(this)` after every FFI call
  - Tool delegates pinned with `GCHandle.Alloc(callback, GCHandleType.Normal)`, stored in `List<GCHandle>`, freed in `Dispose()`
  - Lazy initialization: `SandboxBuilder.Build()` creates `SandboxSafeHandle` but tool registration happens before first `Run()`

- [ ] **3.5** Create `SandboxSnapshot.cs` — wraps `SnapshotSafeHandle`, `IDisposable`

- [ ] **3.6** Create `ToolSchemaBuilder.cs` — auto-generates JSON schema from `Func<T, R>` type parameters via reflection

- [ ] **3.7** Create `SizeParser.cs` — parses "25Mi", "400Mi", "1Gi" to bytes

- [ ] **3.8** Create exception types:
  - `SandboxException` (base)
  - `SandboxTimeoutException : SandboxException`
  - `SandboxPoisonedException : SandboxException`
  - `SandboxPermissionException : SandboxException`
  - `SandboxGuestException : SandboxException`

### Reference files
- PR #292: `src/dotnet-host-api/src/dotnet/Api/` — `SandboxBuilder.cs`, `Sandbox.cs`, `LoadedSandbox.cs`
- Python SDK: `src/sdk/python/core/hyperlight_sandbox/__init__.py`

---

## Phase 4: Extensions.AI Package 🤖

> **Status**: ✅ Complete (CodeExecutionTool + AIFunction integration)
> **Depends on**: Phase 3
> **Project**: `src/sdk/dotnet/core/Extensions.AI/HyperlightSandbox.Extensions.AI.csproj`

### Steps

- [ ] **4.1** Create `HyperlightSandbox.Extensions.AI.csproj`
  - References `HyperlightSandbox.Api`
  - Depends on `Microsoft.Extensions.AI.Abstractions`
  - NuGet metadata: `Hyperlight.HyperlightSandbox.Extensions.AI`

- [ ] **4.2** Create `CodeExecutionTool.cs` — wraps `Sandbox` for agent integration
  ```csharp
  public sealed class CodeExecutionTool : IDisposable
  {
      public CodeExecutionTool(SandboxBuilder? builder = null);

      // Register tools that guest code can call
      public void RegisterTool<TArgs, TResult>(string name, Func<TArgs, TResult> handler);

      // Execute code in sandbox (snapshot/restore per call for clean state)
      public ExecutionResult Execute(string code,
          IDictionary<string, byte[]>? inputs = null);

      // Get as AIFunction for Copilot SDK / MAF integration
      public AIFunction AsAIFunction(string name = "execute_code",
          string description = "Execute code in a secure sandbox");

      public void Dispose();
  }
  ```

- [ ] **4.3** Create `SandboxToolFactory.cs` — helpers for `AIFunctionFactory.Create()` integration

### Reference files
- Python SDK: `CodeExecutionTool` in `src/sdk/python/core/hyperlight_sandbox/__init__.py`
- Copilot SDK: `AIFunctionFactory.Create()` patterns

---

## Phase 5: Examples 📝

> **Status**: ✅ Complete (7 examples: Basic, Tools, FS, Network, Snapshot, Copilot SDK, MAF)
> **Depends on**: Phase 3 (basics), Phase 4 (agent examples)

### Steps

- [ ] **5.1** Create `BasicExample` — run code, capture stdout *(mirrors `python_basics.py`)*
- [ ] **5.2** Create `ToolRegistrationExample` — register tools, guest calls `call_tool()` *(mirrors agent patterns)*
- [ ] **5.3** Create `FilesystemExample` — input/output dirs, temp output *(mirrors `python_filesystem_demo.py`)*
- [ ] **5.4** Create `NetworkExample` — `AllowDomain` + guest HTTP *(mirrors `python_network_demo.py`)*
- [ ] **5.5** Create `SnapshotExample` — snapshot/restore for fast reset
- [ ] **5.6** Create `CopilotSdkExample` — `GitHub.Copilot.SDK` v0.2.2 integration
  - `CopilotClient` + `CreateSessionAsync`
  - Tools via `AIFunctionFactory.Create()`: `execute_code`, `compute`, `fetch_data`
  - System message steers model to use `execute_code`
  - Sandbox snapshot/restore between calls
  - Session event logging
  - *(mirrors `copilot_sdk_tools.py`)*

- [ ] **5.7** Create `AgentFrameworkExample` — `Microsoft.Agents.AI` v1.1.0 integration
  - Agent with `execute_code` tool backed by sandbox
  - Tool registration via sandbox
  - Network allowlist
  - *(mirrors `copilot_agent.py`)*

All examples at `src/sdk/dotnet/core/Examples/{Name}/{Name}.csproj`

---

## Phase 6: Tests 🧪

> **Status**: ✅ Complete (93 tests passing, includes 12 ownership transfer tests)
> **Depends on**: Phase 3

### Steps — ~93+ xUnit tests across 9 test classes

All tests at `src/sdk/dotnet/core/Tests/HyperlightSandbox.Tests/`

- [ ] **6.1** `CoreFunctionalityTests` (~15 tests)
  - Basic run with stdout/stderr capture
  - Exit code propagation (0 and non-zero)
  - Multiple sequential runs
  - Large code input (near MAX_CODE_SIZE)
  - Code exceeding MAX_CODE_SIZE → error
  - Empty code execution
  - Unicode in code and output
  - Async RunAsync

- [ ] **6.2** `ToolRegistrationTests` (~20 tests)
  - Register sync tool, call from guest
  - Register typed `Func<TArgs, TResult>`, auto-serialization
  - Register multiple tools
  - Tool returning various types (string, number, bool, object, array)
  - Tool throwing exception → guest receives error
  - Register after first `Run()` → error
  - Tool schema validation (wrong arg types)
  - Required vs optional args
  - No-arg tools
  - Tool with default values

- [ ] **6.3** `FilesystemTests` (~12 tests)
  - No filesystem (default)
  - Input dir only
  - Temp output only
  - Input + temp output
  - Persistent output dir
  - Get output files after write
  - OutputPath accessor
  - Guest reads input file
  - Guest writes output file
  - Non-existent input dir → error

- [ ] **6.4** `NetworkTests` (~10 tests)
  - No network (default) → guest HTTP fails
  - AllowDomain → guest HTTP succeeds
  - Method filter → only allowed methods
  - Multiple domains
  - CONNECT/TRACE always blocked
  - Invalid domain → error

- [ ] **6.5** `SnapshotRestoreTests` (~10 tests)
  - Take snapshot, restore, clean state
  - Snapshot reused multiple times
  - Disposed snapshot → error
  - Snapshot on disposed sandbox → error
  - Null snapshot → `ArgumentNullException`
  - Async snapshot/restore

- [ ] **6.6** `SandboxBuilderTests` (~8 tests)
  - Default options
  - Custom heap/stack sizes
  - Chained configuration
  - Builder reuse
  - Invalid module path → error
  - Size parsing ("25Mi", "400Mi", invalid)

- [ ] **6.7** `SandboxLifecycleTests` (~10 tests)
  - Create and dispose
  - Using pattern
  - Dispose idempotency
  - Use after dispose → `ObjectDisposedException`
  - Finalizer safety (abandoned sandbox)
  - Thread-affinity violation → exception

- [ ] **6.8** `MemoryLeakTests` (~6 tests)
  - Sandbox creation/disposal loop
  - Run execution loop
  - Tool dispatch loop
  - Snapshot/restore loop
  - Error handling loop
  - Complex mixed operations

- [ ] **6.9** `PackageTests` (separate project, ~2 tests)
  - Install from local NuGet feed
  - Simple execution via installed package

---

## Phase 7: Build System 🔧

> **Status**: ⬜ Not Started
> **Depends on**: Phase 1 + Phase 2 (for building)

### Steps

- [ ] **7.1** Create `src/sdk/dotnet/Justfile`
  ```
  build profile="debug":    Build Rust FFI + dotnet solution
  test profile="debug":     Run xUnit tests
  fmt-check:                dotnet format --verify-no-changes
  fmt-apply:                dotnet format
  analyze:                  dotnet build /p:TreatWarningsAsErrors=true /p:EnforceCodeStyleInBuild=true
  examples:                 Run all example projects
  dist:                     Build NuGet packages → dist/dotnetsdk/
  package-test:             Install + smoke test NuGet packages
  ```

- [ ] **7.2** Update root `Justfile`
  - Add `mod dotnet "src/sdk/dotnet/Justfile"`
  - Wire into `build`, `test`, `fmt`, `fmt-check`, `lint`

- [ ] **7.3** Update root `Cargo.toml`
  - Add `src/sdk/dotnet/ffi` to workspace members

- [ ] **7.4** Create `.editorconfig` for C# formatting rules

---

## Phase 8: NuGet Packaging 📦

> **Status**: ⬜ Not Started
> **Depends on**: Phase 3

### Steps

- [ ] **8.1** Create `Directory.Build.props`
  - Shared: Version, Authors, Copyright, Apache-2.0, repo URL

- [ ] **8.2** Configure NuGet metadata in each `.csproj`
  - `Hyperlight.HyperlightSandbox.PInvoke` — P/Invoke + native libs
  - `Hyperlight.HyperlightSandbox.Api` — high-level API
  - `Hyperlight.HyperlightSandbox.Extensions.AI` — agent integration

- [ ] **8.3** Native library bundling in PInvoke.csproj
  - `runtimes/linux-x64/native/libhyperlight_sandbox_ffi.so`
  - `runtimes/win-x64/native/hyperlight_sandbox_ffi.dll`
  - MSBuild `.targets` file for consumers

- [ ] **8.4** Create `nuget.config` for PackageTests
  - Local dist folder + nuget.org sources

---

## Phase 9: CI + Documentation 📚

> **Status**: ⬜ Not Started
> **Depends on**: Phase 7

### Steps

- [ ] **9.1** Add `.NET 8.0 SDK` setup to CI workflow
- [ ] **9.2** Add `just dotnet fmt-check` to format validation
- [ ] **9.3** Add `just dotnet build` to build pipeline
- [ ] **9.4** Add `just dotnet test` to test pipeline
- [ ] **9.5** Add `just dotnet examples` to example runs
- [ ] **9.6** Create `src/sdk/dotnet/README.md` — architecture, build, API, contributing
- [ ] **9.7** Update root `README.md` — add .NET SDK section
- [ ] **9.8** Update `.github/copilot-instructions.md` — add dotnet commands

---

## File Manifest

### New files to create

| Path | Description |
|------|-------------|
| `src/sdk/dotnet/ffi/Cargo.toml` | Rust FFI crate config |
| `src/sdk/dotnet/ffi/src/lib.rs` | FFI exports (~800-1000 LOC) |
| `src/sdk/dotnet/core/PInvoke/HyperlightSandbox.PInvoke.csproj` | P/Invoke project |
| `src/sdk/dotnet/core/PInvoke/AssemblyInfo.cs` | Runtime marshalling config |
| `src/sdk/dotnet/core/PInvoke/FFIResult.cs` | FFI result struct |
| `src/sdk/dotnet/core/PInvoke/FFIErrorCode.cs` | Error code enum |
| `src/sdk/dotnet/core/PInvoke/SafeHandles.cs` | SafeHandle wrappers |
| `src/sdk/dotnet/core/PInvoke/SafeNativeMethods.cs` | P/Invoke declarations |
| `src/sdk/dotnet/core/PInvoke/SandboxOptions.cs` | Options struct |
| `src/sdk/dotnet/core/PInvoke/ToolCallbackDelegate.cs` | Callback delegate |
| `src/sdk/dotnet/core/PInvoke/build/net8.0/*.targets` | MSBuild targets |
| `src/sdk/dotnet/core/Api/HyperlightSandbox.Api.csproj` | High-level API project |
| `src/sdk/dotnet/core/Api/ExecutionResult.cs` | Result record |
| `src/sdk/dotnet/core/Api/SandboxBuilder.cs` | Fluent builder |
| `src/sdk/dotnet/core/Api/Sandbox.cs` | Main API class |
| `src/sdk/dotnet/core/Api/SandboxSnapshot.cs` | Snapshot wrapper |
| `src/sdk/dotnet/core/Api/ToolSchemaBuilder.cs` | Schema generator |
| `src/sdk/dotnet/core/Api/SizeParser.cs` | Size string parser |
| `src/sdk/dotnet/core/Api/Exceptions.cs` | Exception hierarchy |
| `src/sdk/dotnet/core/Extensions.AI/HyperlightSandbox.Extensions.AI.csproj` | AI extensions |
| `src/sdk/dotnet/core/Extensions.AI/CodeExecutionTool.cs` | Agent tool wrapper |
| `src/sdk/dotnet/core/Extensions.AI/SandboxToolFactory.cs` | AIFunction helpers |
| `src/sdk/dotnet/core/Examples/BasicExample/` | Basic usage sample |
| `src/sdk/dotnet/core/Examples/ToolRegistrationExample/` | Tool registration sample |
| `src/sdk/dotnet/core/Examples/FilesystemExample/` | Filesystem sample |
| `src/sdk/dotnet/core/Examples/NetworkExample/` | Network sample |
| `src/sdk/dotnet/core/Examples/SnapshotExample/` | Snapshot sample |
| `src/sdk/dotnet/core/Examples/CopilotSdkExample/` | Copilot SDK sample |
| `src/sdk/dotnet/core/Examples/AgentFrameworkExample/` | MAF sample |
| `src/sdk/dotnet/core/Tests/HyperlightSandbox.Tests/` | Main test project |
| `src/sdk/dotnet/core/Tests/HyperlightSandbox.PackageTests/` | NuGet package tests |
| `src/sdk/dotnet/core/HyperlightSandbox.sln` | Solution file |
| `src/sdk/dotnet/core/.editorconfig` | C# formatting rules |
| `src/sdk/dotnet/Directory.Build.props` | Shared NuGet metadata |
| `src/sdk/dotnet/Justfile` | Build recipes |
| `src/sdk/dotnet/README.md` | Documentation |

### Existing files to modify

| Path | Change |
|------|--------|
| `Cargo.toml` | Add `src/sdk/dotnet/ffi` to workspace members |
| `Justfile` | Add dotnet module, wire into build/test/fmt/lint |
| `.github/copilot-instructions.md` | Add dotnet build/test commands |
| `README.md` | Add .NET SDK section |

---

## Verification Checklist

Before declaring done:

- [ ] `just fmt` passes (all Rust + .NET formatting clean)
- [ ] `just build` compiles Rust FFI crate + .NET solution
- [ ] `just dotnet test` — 93+ xUnit tests pass
- [ ] `just dotnet examples` — all 7 examples run successfully
- [ ] `just dotnet fmt-check` — no formatting violations
- [ ] `just dotnet analyze` — Roslyn analyzers clean (warnings as errors)
- [ ] `just dotnet package-test` — NuGet packages install and work
- [ ] Manual: example outputs are correct
- [ ] No `expect`/`unwrap` in production Rust code
- [ ] No `unsafe` in public C# API surface
- [ ] All SafeHandles properly freed
- [ ] GC.KeepAlive barriers on all FFI call sites
- [ ] Tool callback delegates properly pinned

---

## Architecture Diagram

```
┌─────────────────────────────────────────────────────┐
│                    .NET Application                  │
│                                                      │
│  ┌──────────────────┐  ┌───────────────────────────┐│
│  │ HyperlightSandbox│  │ HyperlightSandbox         ││
│  │     .Api          │  │   .Extensions.AI          ││
│  │                   │  │                           ││
│  │  Sandbox          │  │  CodeExecutionTool        ││
│  │  SandboxBuilder   │  │  SandboxToolFactory       ││
│  │  ExecutionResult  │  │    → AIFunction           ││
│  │  SandboxSnapshot  │  │                           ││
│  └────────┬──────────┘  └────────┬──────────────────┘│
│           │                      │                    │
│  ┌────────▼──────────────────────▼──────────────────┐│
│  │       HyperlightSandbox.PInvoke                  ││
│  │                                                   ││
│  │  SafeNativeMethods  SafeHandles  FFIResult       ││
│  │  [LibraryImport]    GCHandle     ThrowIfError()  ││
│  └────────────────────────┬──────────────────────────┘│
│                           │ P/Invoke                  │
└───────────────────────────┼───────────────────────────┘
                            │
          ┌─────────────────▼─────────────────┐
          │  hyperlight_sandbox_ffi (cdylib)   │
          │                                    │
          │  FFI exports (extern "C")          │
          │  Box::into_raw / from_raw          │
          │  CString / JSON marshalling        │
          └─────────────────┬──────────────────┘
                            │
          ┌─────────────────▼─────────────────┐
          │  hyperlight-sandbox (core)         │
          │  + hyperlight-wasm-sandbox         │
          │                                    │
          │  Sandbox<Wasm>                     │
          │  ToolRegistry / CapFs / Network    │
          │  Snapshot / Restore                │
          └────────────────────────────────────┘
```

---

## Changelog

| Date | Change |
|------|--------|
| 2026-04-13 | Initial plan created from PR #292 review + Python SDK analysis |
| 2026-04-13 | Phase 1 complete: FFI crate with 68 tests, clean compile, formatted |
| 2026-04-13 | Phase 2 complete: P/Invoke layer - 7 source files, auto-builds Rust, zero warnings |
| 2026-04-13 | Phase 3 complete: API layer - Sandbox, SandboxBuilder, tool registration, exceptions, snapshots |
| 2026-04-13 | Phase 6 complete: 93 xUnit tests - ownership transfers, GC stress, memory leaks, lifecycle |
| 2026-04-13 | Phase 4 complete: Extensions.AI - CodeExecutionTool with AIFunction adapter |
| 2026-04-13 | Phase 5 complete: 7 examples including Copilot SDK and MAF integration |
