using System.Runtime.InteropServices;
using System.Text.Json;
using HyperlightSandbox.PInvoke;

namespace HyperlightSandbox.Api;

/// <summary>
/// A secure sandbox for executing guest code with configurable tools,
/// filesystem access, and network permissions.
/// </summary>
/// <remarks>
/// <para>
/// <b>Thread safety</b>: Sandbox instances are <c>Send</c> but not <c>Sync</c> —
/// they can be moved between threads but must not be accessed concurrently
/// from multiple threads. All public methods acquire an internal lock to
/// enforce this. If you need parallel execution, create one sandbox per thread.
/// </para>
/// <para>
/// <b>Lifecycle</b>:
/// <list type="number">
/// <item>Create via <see cref="SandboxBuilder"/>.</item>
/// <item>Register tools via <see cref="RegisterTool{TArgs, TResult}"/>
///       (must be done before the first <see cref="Run"/>).</item>
/// <item>Configure network via <see cref="AllowDomain"/>.</item>
/// <item>Execute code via <see cref="Run"/> (triggers lazy initialization
///       on first call).</item>
/// <item>Dispose when done.</item>
/// </list>
/// </para>
/// </remarks>
public sealed class Sandbox : IDisposable
{
    /// <summary>Maximum allowed code size (10 MiB).</summary>
    public const int MaxCodeSize = 10 * 1024 * 1024;

    private readonly SandboxSafeHandle _handle;
    private readonly object _gate = new();
    private readonly List<GCHandle> _pinnedDelegates = [];
    private bool _disposed;

    /// <summary>
    /// Creates a new sandbox. Use <see cref="SandboxBuilder"/> instead of
    /// calling this directly.
    /// </summary>
    internal Sandbox(
        string? modulePath,
        ulong heapSize,
        ulong stackSize,
        string? inputDir,
        string? outputDir,
        bool tempOutput,
        SandboxBackend backend = SandboxBackend.Wasm)
    {
        // Pin the module path string for the FFI call duration (null for JS backend).
        var modulePathPtr = modulePath != null
            ? Marshal.StringToCoTaskMemUTF8(modulePath)
            : IntPtr.Zero;
        try
        {
            var options = new FFISandboxOptions
            {
                module_path = modulePathPtr,
                heap_size = heapSize,
                stack_size = stackSize,
                backend = (uint)backend,
            };

            var result = SafeNativeMethods.hyperlight_sandbox_create(options);
            result.ThrowIfError();

            _handle = new SandboxSafeHandle(result.value);
        }
        finally
        {
            if (modulePathPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(modulePathPtr);
            }
        }

        // Apply optional configuration.
        // Note: GC.KeepAlive(this) is not needed in the constructor — the
        // object cannot be finalized while its constructor is still running.
        if (inputDir != null)
        {
            var r = SafeNativeMethods.hyperlight_sandbox_set_input_dir(_handle, inputDir);
            r.ThrowIfError();
        }

        if (outputDir != null)
        {
            var r = SafeNativeMethods.hyperlight_sandbox_set_output_dir(_handle, outputDir);
            r.ThrowIfError();
        }

        if (tempOutput)
        {
            var r = SafeNativeMethods.hyperlight_sandbox_set_temp_output(_handle, true);
            r.ThrowIfError();
        }
    }

    // -----------------------------------------------------------------------
    // Tool registration
    // -----------------------------------------------------------------------

    /// <summary>
    /// Registers a typed tool that guest code can invoke via
    /// <c>call_tool("name", ...)</c>.
    /// </summary>
    /// <typeparam name="TArgs">
    /// The argument type. Public properties define the tool's parameter schema.
    /// </typeparam>
    /// <typeparam name="TResult">
    /// The return type. Serialized to JSON for the guest.
    /// </typeparam>
    /// <param name="name">Tool name (must be unique).</param>
    /// <param name="handler">
    /// Function invoked when the guest calls this tool. Receives deserialized
    /// arguments. The return value is serialized to JSON for the guest.
    /// </param>
    /// <exception cref="InvalidOperationException">
    /// Thrown if called after the first <see cref="Run"/> call.
    /// </exception>
    /// <remarks>
    /// <para>
    /// The <paramref name="handler"/> delegate is pinned in memory via
    /// <see cref="GCHandle"/> for the lifetime of this sandbox. This prevents
    /// the GC from collecting it while Rust holds the function pointer.
    /// </para>
    /// </remarks>
    public void RegisterTool<TArgs, TResult>(string name, Func<TArgs, TResult> handler)
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            ArgumentException.ThrowIfNullOrWhiteSpace(name);
            ArgumentNullException.ThrowIfNull(handler);

            // Build schema from TArgs properties.
            var schemaJson = ToolSchemaBuilder.BuildSchema<TArgs>();

            // Create the unmanaged callback that bridges .NET ↔ Rust.
            ToolCallbackDelegate callback = (argsJsonPtr) =>
            {
                try
                {
                    // Read the JSON args from Rust.
                    var argsJson = Marshal.PtrToStringUTF8(argsJsonPtr);
                    if (argsJson == null)
                    {
                        return MarshalErrorResult("Tool callback received null arguments");
                    }

                    // Deserialize to the typed args.
                    var args = JsonSerializer.Deserialize<TArgs>(argsJson);
                    if (args == null)
                    {
                        return MarshalErrorResult(
                            $"Failed to deserialize arguments to {typeof(TArgs).Name}");
                    }

                    // Invoke the user's handler.
                    var result = handler(args);

                    // Serialize the result to JSON.
                    var resultJson = JsonSerializer.Serialize(result);
                    return Marshal.StringToCoTaskMemUTF8(resultJson);
                }
#pragma warning disable CA1031 // Catch general exception — intentional in FFI callback to prevent unhandled exceptions crossing the native boundary
                catch (Exception ex)
#pragma warning restore CA1031
                {
                    return MarshalErrorResult(ex.Message);
                }
            };

            // Pin the delegate so GC doesn't collect it while Rust holds the fn ptr.
            // This is CRITICAL — without pinning, the function pointer becomes dangling
            // after a GC cycle, causing SIGSEGV when Rust invokes it.
            var gcHandle = GCHandle.Alloc(callback);
            _pinnedDelegates.Add(gcHandle);

            var fnPtr = Marshal.GetFunctionPointerForDelegate(callback);
            var result = SafeNativeMethods.hyperlight_sandbox_register_tool(
                _handle, name, schemaJson, fnPtr);

            GC.KeepAlive(this);
            result.ThrowIfError();
        } // lock
    }

    /// <summary>
    /// Registers a typed tool whose handler is asynchronous.
    /// </summary>
    /// <typeparam name="TArgs">
    /// The argument type. Public properties define the tool's parameter schema.
    /// </typeparam>
    /// <typeparam name="TResult">
    /// The return type. Serialized to JSON for the guest.
    /// </typeparam>
    /// <param name="name">Tool name (must be unique).</param>
    /// <param name="handler">
    /// Async function invoked when the guest calls this tool. Receives
    /// deserialized arguments. The return value is serialized to JSON for
    /// the guest.
    /// </param>
    /// <remarks>
    /// <para>
    /// The underlying FFI callback is synchronous — the async handler is
    /// blocked on at the interop boundary via <c>GetAwaiter().GetResult()</c>.
    /// This is safe because FFI callbacks run on threads without a
    /// <see cref="System.Threading.SynchronizationContext"/>.
    /// </para>
    /// </remarks>
    public void RegisterToolAsync<TArgs, TResult>(string name, Func<TArgs, Task<TResult>> handler)
    {
        // Wrap the async handler into a sync handler that blocks at the FFI boundary.
        RegisterTool<TArgs, TResult>(name, args => handler(args).GetAwaiter().GetResult());
    }

    /// <summary>
    /// Registers a tool with raw JSON input/output.
    /// </summary>
    /// <param name="name">Tool name.</param>
    /// <param name="handler">
    /// Function receiving a JSON string and returning a JSON string.
    /// Return <c>{"error": "message"}</c> to signal an error to the guest.
    /// </param>
    public void RegisterTool(string name, Func<string, string> handler)
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            ArgumentException.ThrowIfNullOrWhiteSpace(name);
            ArgumentNullException.ThrowIfNull(handler);

            ToolCallbackDelegate callback = (argsJsonPtr) =>
            {
                try
                {
                    var argsJson = Marshal.PtrToStringUTF8(argsJsonPtr) ?? "{}";
                    var resultJson = handler(argsJson);
                    return Marshal.StringToCoTaskMemUTF8(resultJson);
                }
#pragma warning disable CA1031 // Catch general exception — intentional in FFI callback
                catch (Exception ex)
#pragma warning restore CA1031
                {
                    return MarshalErrorResult(ex.Message);
                }
            };

            var gcHandle = GCHandle.Alloc(callback);
            _pinnedDelegates.Add(gcHandle);

            var fnPtr = Marshal.GetFunctionPointerForDelegate(callback);
            var result = SafeNativeMethods.hyperlight_sandbox_register_tool(
                _handle, name, null, fnPtr);

            GC.KeepAlive(this);
            result.ThrowIfError();
        } // lock
    }

    /// <summary>
    /// Registers a raw JSON tool whose handler is asynchronous.
    /// </summary>
    /// <param name="name">Tool name.</param>
    /// <param name="handler">
    /// Async function receiving a JSON string and returning a JSON string.
    /// Return <c>{"error": "message"}</c> to signal an error to the guest.
    /// </param>
    /// <remarks>
    /// <para>
    /// The underlying FFI callback is synchronous — the async handler is
    /// blocked on at the interop boundary via <c>GetAwaiter().GetResult()</c>.
    /// This is safe because FFI callbacks run on threads without a
    /// <see cref="System.Threading.SynchronizationContext"/>.
    /// </para>
    /// </remarks>
    public void RegisterToolAsync(string name, Func<string, Task<string>> handler)
    {
        // Wrap the async handler into a sync handler that blocks at the FFI boundary.
        RegisterTool(name, (string json) => handler(json).GetAwaiter().GetResult());
    }

    // -----------------------------------------------------------------------
    // Code execution
    // -----------------------------------------------------------------------

    /// <summary>
    /// Executes guest code in the sandbox.
    /// </summary>
    /// <param name="code">The code to execute.</param>
    /// <returns>The execution result containing stdout, stderr, and exit code.</returns>
    /// <exception cref="ArgumentException">
    /// Thrown if <paramref name="code"/> is null/empty or exceeds
    /// <see cref="MaxCodeSize"/>.
    /// </exception>
    /// <exception cref="SandboxException">Thrown if execution fails.</exception>
    /// <remarks>
    /// The first call triggers lazy initialization of the sandbox runtime
    /// (building the Wasm sandbox, registering tools, applying network
    /// permissions). Subsequent calls reuse the initialized runtime.
    /// </remarks>
    public ExecutionResult Run(string code)
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            ArgumentException.ThrowIfNullOrWhiteSpace(code);

            if (System.Text.Encoding.UTF8.GetByteCount(code) > MaxCodeSize)
            {
                throw new ArgumentException(
                    $"Code exceeds maximum size (max {MaxCodeSize} bytes).",
                    nameof(code));
            }

            var result = SafeNativeMethods.hyperlight_sandbox_run(_handle, code);
            GC.KeepAlive(this);
            result.ThrowIfError();

            var json = FFIResult.StringFromPtr(result.value);
            if (json == null)
            {
                throw new SandboxException("Execution returned null result.");
            }

            var execResult = JsonSerializer.Deserialize<ExecutionResultDto>(json)
                ?? throw new SandboxException("Failed to deserialize execution result.");

            return new ExecutionResult(
                execResult.stdout ?? string.Empty,
                execResult.stderr ?? string.Empty,
                execResult.exit_code);
        } // lock
    }

    /// <summary>
    /// Runs <see cref="Run"/> on the thread pool.
    /// Only use if you need to free up the calling thread (e.g., UI apps).
    /// For ASP.NET Core, prefer calling <see cref="Run"/> directly.
    /// </summary>
    /// <remarks>
    /// The cancellation token prevents scheduling of the task but cannot
    /// cancel an in-progress FFI call. Once <see cref="Run"/> starts
    /// executing in the native layer, it will run to completion.
    /// </remarks>
    /// <param name="code">The code to execute.</param>
    /// <param name="cancellationToken">Token to prevent scheduling (does not cancel in-progress execution).</param>
    public Task<ExecutionResult> RunAsync(string code, CancellationToken cancellationToken = default)
        => Task.Run(() => Run(code), cancellationToken);

    // -----------------------------------------------------------------------
    // Network
    // -----------------------------------------------------------------------

    /// <summary>
    /// Adds a domain to the network allowlist.
    /// </summary>
    /// <param name="target">
    /// URL or domain (e.g. <c>"https://httpbin.org"</c>).
    /// </param>
    /// <param name="methods">
    /// Optional HTTP methods to allow (e.g. <c>["GET", "POST"]</c>).
    /// <c>null</c> allows all methods.
    /// </param>
    public void AllowDomain(string target, IReadOnlyList<string>? methods = null)
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            ArgumentException.ThrowIfNullOrWhiteSpace(target);

            string? methodsJson = methods != null
                ? JsonSerializer.Serialize(methods)
                : null;

            var result = SafeNativeMethods.hyperlight_sandbox_allow_domain(
                _handle, target, methodsJson);
            GC.KeepAlive(this);
            result.ThrowIfError();
        } // lock
    }

    // -----------------------------------------------------------------------
    // Filesystem
    // -----------------------------------------------------------------------

    /// <summary>
    /// Lists filenames written to the output directory by guest code.
    /// </summary>
    /// <returns>List of filenames.</returns>
    /// <exception cref="InvalidOperationException">
    /// Thrown if the sandbox has not been initialized (no <see cref="Run"/>
    /// call yet).
    /// </exception>
    public IReadOnlyList<string> GetOutputFiles()
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            var result = SafeNativeMethods.hyperlight_sandbox_get_output_files(_handle);
            GC.KeepAlive(this);
            result.ThrowIfError();

            var json = FFIResult.StringFromPtr(result.value) ?? "[]";
            return JsonSerializer.Deserialize<List<string>>(json) ?? [];
        } // lock
    }

    /// <summary>
    /// Returns the host filesystem path of the output directory, or
    /// <c>null</c> if no output directory is configured.
    /// </summary>
    public string? OutputPath
    {
        get
        {
            lock (_gate)
            {
                ThrowIfDisposed();

                var result = SafeNativeMethods.hyperlight_sandbox_output_path(_handle);
                GC.KeepAlive(this);
                result.ThrowIfError();

                return FFIResult.StringFromPtr(result.value);
            } // lock
        }
    }

    // -----------------------------------------------------------------------
    // Snapshot / Restore
    // -----------------------------------------------------------------------

    /// <summary>
    /// Takes a snapshot of the current sandbox state.
    /// </summary>
    /// <returns>A snapshot that can be passed to <see cref="Restore"/>.</returns>
    /// <remarks>
    /// The sandbox must be initialized (at least one <see cref="Run"/> call).
    /// The returned snapshot must be disposed when no longer needed.
    /// </remarks>
    public SandboxSnapshot Snapshot()
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            var result = SafeNativeMethods.hyperlight_sandbox_snapshot(_handle);
            GC.KeepAlive(this);
            result.ThrowIfError();

            var snapshotHandle = new SnapshotSafeHandle(result.value);
            return new SandboxSnapshot(snapshotHandle);
        } // lock
    }

    /// <summary>
    /// Runs <see cref="Snapshot"/> on the thread pool.
    /// </summary>
    /// <param name="cancellationToken">Token to prevent scheduling.</param>
    public Task<SandboxSnapshot> SnapshotAsync(CancellationToken cancellationToken = default)
        => Task.Run(Snapshot, cancellationToken);

    /// <summary>
    /// Restores the sandbox to a previously captured snapshot state.
    /// </summary>
    /// <param name="snapshot">
    /// The snapshot to restore from. This handle is NOT consumed and can
    /// be reused.
    /// </param>
    /// <exception cref="ArgumentNullException">
    /// Thrown if <paramref name="snapshot"/> is null.
    /// </exception>
    /// <exception cref="ObjectDisposedException">
    /// Thrown if <paramref name="snapshot"/> has been disposed.
    /// </exception>
    public void Restore(SandboxSnapshot snapshot)
    {
        lock (_gate)
        {
            ThrowIfDisposed();

            ArgumentNullException.ThrowIfNull(snapshot);

            if (snapshot.IsDisposed)
            {
#pragma warning disable CA1513 // ThrowIf not applicable — checking external object's disposed state
                throw new ObjectDisposedException(nameof(SandboxSnapshot),
                    "The snapshot has already been disposed.");
#pragma warning restore CA1513
            }

            var result = SafeNativeMethods.hyperlight_sandbox_restore(_handle, snapshot.Handle);
            GC.KeepAlive(this);
            GC.KeepAlive(snapshot);
            result.ThrowIfError();
        } // lock
    }

    /// <summary>
    /// Runs <see cref="Restore"/> on the thread pool.
    /// </summary>
    /// <param name="snapshot">The snapshot to restore from.</param>
    /// <param name="cancellationToken">Token to prevent scheduling.</param>
    public Task RestoreAsync(SandboxSnapshot snapshot, CancellationToken cancellationToken = default)
        => Task.Run(() => Restore(snapshot), cancellationToken);

    // -----------------------------------------------------------------------
    // Dispose
    // -----------------------------------------------------------------------

    /// <summary>
    /// Releases the sandbox and all associated native resources.
    /// </summary>
    /// <remarks>
    /// The native sandbox handle is released before pinned tool callback
    /// delegates are freed. This keeps callbacks valid for the full native
    /// drop path if the backend ever invokes them during cleanup.
    /// </remarks>
    public void Dispose()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;

            ReleaseNativeHandle();
            FreePinnedDelegates();

            GC.SuppressFinalize(this);
        } // lock
    }

    /// <summary>
    /// Destructor — ensures the native handle is released before pinned
    /// GCHandles are freed even if the user forgets to call <see cref="Dispose"/>.
    /// </summary>
    ~Sandbox()
    {
        ReleaseNativeHandle();
        FreePinnedDelegates();
    }

    /// <summary>
    /// Releases the native sandbox handle. Safe to call multiple times.
    /// </summary>
    private void ReleaseNativeHandle()
    {
        if (!_handle.IsInvalid)
        {
            _handle.Dispose();
        }
    }

    /// <summary>
    /// Frees all pinned tool callback delegates.
    /// Safe to call multiple times (idempotent).
    /// </summary>
    private void FreePinnedDelegates()
    {
        foreach (var gcHandle in _pinnedDelegates)
        {
            if (gcHandle.IsAllocated)
            {
                gcHandle.Free();
            }
        }

        _pinnedDelegates.Clear();
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// <summary>
    /// Throws <see cref="ObjectDisposedException"/> if this sandbox has been
    /// disposed. Must be called inside <c>lock (_gate)</c>.
    /// </summary>
    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
    }

    /// <summary>
    /// Marshals an error message into a CoTaskMem UTF-8 JSON string for
    /// returning from a tool callback.
    /// </summary>
    private static IntPtr MarshalErrorResult(string message)
    {
        var errorJson = JsonSerializer.Serialize(new { error = message });
        return Marshal.StringToCoTaskMemUTF8(errorJson);
    }

    /// <summary>
    /// DTO for deserializing the JSON execution result from Rust.
    /// </summary>
    [System.Diagnostics.CodeAnalysis.SuppressMessage(
        "Performance", "CA1812:Avoid uninstantiated internal classes",
        Justification = "Instantiated by System.Text.Json during deserialization")]
    private sealed class ExecutionResultDto
    {
        public string? stdout { get; set; }
        public string? stderr { get; set; }
        public int exit_code { get; set; }
    }
}
