using System.ComponentModel;
using System.Text.Json;
using Microsoft.Extensions.AI;

namespace HyperlightSandbox.Extensions.AI;

/// <summary>
/// A high-level wrapper around <see cref="Api.Sandbox"/> designed for agent
/// framework integration.
///
/// Provides a self-contained code execution tool that:
/// <list type="bullet">
/// <item>Manages sandbox lifecycle (create, snapshot, restore, dispose).</item>
/// <item>Provides snapshot/restore per execution for clean state.</item>
/// <item>Exposes itself as an <see cref="AIFunction"/> for use with
///       GitHub Copilot SDK and Microsoft Agent Framework.</item>
/// </list>
/// </summary>
/// <remarks>
/// <example>
/// <code>
/// var tool = new CodeExecutionTool(
///     new SandboxBuilder()
///         .WithModulePath("python-sandbox.aot")
///         .WithTempOutput());
///
/// tool.RegisterTool&lt;AddArgs, AddResult&gt;("add",
///     args =&gt; new AddResult { Sum = args.A + args.B });
///
/// // Use with Copilot SDK:
/// var session = await client.CreateSessionAsync(new SessionConfig
/// {
///     Tools = [tool.AsAIFunction()],
/// });
/// </code>
/// </example>
/// </remarks>
public sealed class CodeExecutionTool : IDisposable
{
    private const string WasmInitializationCode = "None";
    private const string JavaScriptInitializationCode = "void 0;";

    private readonly Api.Sandbox _sandbox;
    private readonly string _initializationCode;
    private Api.SandboxSnapshot? _snapshot;
    private bool _initialized;
    private bool _disposed;
    private readonly object _gate = new();

    /// <summary>
    /// Creates a new code execution tool from a pre-configured builder.
    /// </summary>
    /// <param name="builder">
    /// A <see cref="Api.SandboxBuilder"/> configured with the desired module,
    /// heap/stack sizes, and filesystem options.
    /// </param>
    public CodeExecutionTool(Api.SandboxBuilder builder)
    {
        ArgumentNullException.ThrowIfNull(builder);
        _initializationCode = InitializationCodeFor(builder.Backend);
        _sandbox = builder.Build();
    }

    /// <summary>
    /// Registers a typed tool that guest code can invoke via <c>call_tool()</c>.
    /// Must be called before the first <see cref="Execute"/>.
    /// </summary>
    public void RegisterTool<TArgs, TResult>(string name, Func<TArgs, TResult> handler)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _sandbox.RegisterTool(name, handler);
        }
    }

    /// <summary>
    /// Registers a raw JSON tool that guest code can invoke.
    /// </summary>
    public void RegisterTool(string name, Func<string, string> handler)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _sandbox.RegisterTool(name, handler);
        }
    }

    /// <summary>
    /// Registers a typed tool whose handler is asynchronous.
    /// Must be called before the first <see cref="Execute"/>.
    /// </summary>
    public void RegisterToolAsync<TArgs, TResult>(string name, Func<TArgs, Task<TResult>> handler)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _sandbox.RegisterToolAsync(name, handler);
        }
    }

    /// <summary>
    /// Registers a raw JSON tool whose handler is asynchronous.
    /// </summary>
    public void RegisterToolAsync(string name, Func<string, Task<string>> handler)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _sandbox.RegisterToolAsync(name, handler);
        }
    }

    /// <summary>
    /// Adds a domain to the network allowlist.
    /// </summary>
    public void AllowDomain(string target, IReadOnlyList<string>? methods = null)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);
            _sandbox.AllowDomain(target, methods);
        }
    }

    /// <summary>
    /// Executes code in the sandbox with automatic snapshot/restore for
    /// clean state between calls.
    /// </summary>
    /// <remarks>
    /// On the first call, the sandbox is lazily initialized by running a
    /// no-op to trigger runtime setup, then a "warm" snapshot is taken of the
    /// clean post-init state. Subsequent calls restore to this clean snapshot
    /// before executing user code, preventing side effects from leaking.
    /// </remarks>
    /// <param name="code">The code to execute.</param>
    /// <returns>The execution result.</returns>
    public Api.ExecutionResult Execute(string code)
    {
        lock (_gate)
        {
            ObjectDisposedException.ThrowIf(_disposed, this);

            if (!_initialized)
            {
                // Initialize the sandbox runtime with a no-op, then snapshot
                // the CLEAN state before any user code pollutes it.
                _sandbox.Run(_initializationCode);
                _snapshot = _sandbox.Snapshot();
                _initialized = true;
            }

            // Restore to clean post-init state before executing user code.
            if (_snapshot != null)
            {
                _sandbox.Restore(_snapshot);
            }

            return _sandbox.Run(code);
        } // lock
    }

    /// <summary>
    /// Returns this tool as an <see cref="AIFunction"/> for use with
    /// GitHub Copilot SDK or Microsoft Agent Framework.
    /// </summary>
    /// <param name="name">Tool name exposed to the LLM (default: "execute_code").</param>
    /// <param name="description">
    /// Tool description shown to the LLM (default: standard code execution description).
    /// </param>
    /// <returns>An <see cref="AIFunction"/> ready for agent registration.</returns>
    public AIFunction AsAIFunction(
        string name = "execute_code",
        string? description = null)
    {
        ObjectDisposedException.ThrowIf(_disposed, this);

        description ??= "Execute code in a secure sandboxed environment. " +
            "The code runs in an isolated sandbox with no access to the host " +
            "system except for explicitly registered tools and allowed domains.";

        return AIFunctionFactory.Create(
            ([Description("The code to execute in the sandbox")] string code) =>
            {
                var result = Execute(code);
                return JsonSerializer.Serialize(new
                {
                    stdout = result.Stdout,
                    stderr = result.Stderr,
                    exit_code = result.ExitCode,
                    success = result.Success,
                });
            },
            name,
            description);
    }

    /// <summary>
    /// Releases the sandbox and all associated resources.
    /// </summary>
    public void Dispose()
    {
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            _snapshot?.Dispose();
            _sandbox.Dispose();
        } // lock
        GC.SuppressFinalize(this);
    }

    /// <summary>
    /// Destructor — ensures the snapshot is freed if Dispose is not called.
    /// The underlying <see cref="Api.Sandbox"/> has its own finalizer via
    /// <see cref="System.Runtime.InteropServices.SafeHandle"/> — we must NOT
    /// call <c>_sandbox.Dispose()</c> here because it acquires a lock, which
    /// is forbidden in finalizer context (deadlocks the finalizer thread).
    /// </summary>
    ~CodeExecutionTool()
    {
        // Only free what WE own. The sandbox cleans up via its own finalizer.
        _snapshot?.Dispose();
    }

    internal static string InitializationCodeFor(Api.SandboxBackend backend) => backend switch
    {
        Api.SandboxBackend.Wasm => WasmInitializationCode,
        Api.SandboxBackend.JavaScript => JavaScriptInitializationCode,
        _ => throw new ArgumentOutOfRangeException(nameof(backend), backend, "Unknown sandbox backend."),
    };
}
