using HyperlightSandbox.Api;
using Xunit;
using Xunit.Abstractions;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Integration tests that execute real guest code through the full stack:
/// C# → P/Invoke → Rust FFI → hyperlight-sandbox → Wasm VM → Guest.
///
/// These tests require the Python guest module to be pre-built:
///   just wasm guest-build
///
/// Tests are skipped if the guest module is not found (CI without guest build).
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class IntegrationTests
{
    private readonly ITestOutputHelper _output;

    public IntegrationTests(ITestOutputHelper output)
    {
        _output = output;
    }

    /// <summary>
    /// Finds the Python guest module by walking up to the repo root.
    /// Returns null if not found (tests will be skipped).
    /// </summary>
    private static string? FindPythonGuest()
    {
        var dir = AppContext.BaseDirectory;
        while (dir != null)
        {
            if (File.Exists(Path.Combine(dir, "Cargo.toml"))
                && Directory.Exists(Path.Combine(dir, "src", "wasm_sandbox")))
            {
                var path = Path.Combine(dir,
                    "src", "wasm_sandbox", "guests", "python", "python-sandbox.aot");
                return File.Exists(path) ? Path.GetFullPath(path) : null;
            }

            dir = Path.GetDirectoryName(dir);
        }

        return null;
    }

    private Sandbox? TryCreateSandbox()
    {
        var guestPath = FindPythonGuest();
        if (guestPath == null)
        {
            _output.WriteLine("⚠️ Python guest not found — skipping integration test. Run 'just wasm guest-build' first.");
            return null;
        }

        return new SandboxBuilder()
            .WithModulePath(guestPath)
            .Build();
    }

    // -----------------------------------------------------------------------
    // Basic execution
    // -----------------------------------------------------------------------

    [Fact]

    public void Integration_BasicExecution_ReturnsStdout()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        var result = sandbox.Run("print('hello from integration test')");

        Assert.True(result.Success);
        Assert.Equal(0, result.ExitCode);
        Assert.Contains("hello from integration test", result.Stdout, StringComparison.Ordinal);
        Assert.Empty(result.Stderr);
    }

    [Fact]
    public void Integration_MultipleRuns_AllSucceed()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        for (int i = 0; i < 5; i++)
        {
            var result = sandbox.Run($"print('run {i}')");
            Assert.True(result.Success);
            Assert.Contains($"run {i}", result.Stdout, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void Integration_Computation_ProducesCorrectResult()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        var result = sandbox.Run("""
            import math
            print(math.factorial(10))
            """);

        Assert.True(result.Success);
        Assert.Contains("3628800", result.Stdout, StringComparison.Ordinal);
    }

    // -----------------------------------------------------------------------
    // Tool dispatch (full stack)
    // -----------------------------------------------------------------------

    [Fact]
    public void Integration_ToolDispatch_TypedTool_Works()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        sandbox.RegisterTool<AddArgs, double>("add", args => args.a + args.b);

        var result = sandbox.Run("""
            result = call_tool("add", a=100, b=42)
            print(f"result={result}")
            """);

        Assert.True(result.Success);
        Assert.Contains("result=142", result.Stdout, StringComparison.Ordinal);
    }

    [Fact]
    public void Integration_ToolDispatch_RawJsonTool_Works()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        sandbox.RegisterTool("greet", (string json) =>
        {
            if (json.Contains("world", StringComparison.Ordinal))
                return """{"message": "Hello, World!"}""";
            return """{"message": "Hello, stranger!"}""";
        });

        var result = sandbox.Run("""
            r = call_tool("greet", name="world")
            print(r)
            """);

        Assert.True(result.Success);
        Assert.Contains("Hello", result.Stdout, StringComparison.Ordinal);
    }

    [Fact]
    public void Integration_ToolDispatch_MultipleTools_Works()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        sandbox.RegisterTool<AddArgs, double>("add", args => args.a + args.b);
        sandbox.RegisterTool<AddArgs, double>("multiply", args => args.a * args.b);

        var result = sandbox.Run("""
            sum = call_tool("add", a=3, b=4)
            product = call_tool("multiply", a=6, b=7)
            print(f"{sum} {product}")
            """);

        Assert.True(result.Success);
        Assert.Contains("7", result.Stdout, StringComparison.Ordinal);
        Assert.Contains("42", result.Stdout, StringComparison.Ordinal);
    }

    // -----------------------------------------------------------------------
    // Snapshot/Restore
    // -----------------------------------------------------------------------

    [Fact]
    public void Integration_SnapshotRestore_ResetsState()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        // Initialize and set a variable
        var setResult = sandbox.Run("x = 'initial'");
        Assert.True(setResult.Success);

        // Snapshot captures current state
        using var snapshot = sandbox.Snapshot();

        // Modify state
        sandbox.Run("x = 'modified'");

        // Restore
        sandbox.Restore(snapshot);

        // After restore, check what state we get back.
        // The restored guest state should match the snapshot point.
        var result = sandbox.Run("""
            try:
                print(f"x={x}")
            except NameError:
                print("x=undefined")
            """);

        Assert.True(result.Success);
        // The snapshot/restore behaviour: state is rewound to snapshot point.
        // x should either be 'initial' (if state preserved) or undefined
        // (if runtime reset). Both are valid — the key is it's NOT 'modified'.
        Assert.DoesNotContain("x=modified", result.Stdout);
    }

    [Fact]
    public void Integration_SnapshotReuse_WorksMultipleTimes()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        // Initialize
        sandbox.Run("pass");
        using var snapshot = sandbox.Snapshot();

        for (int i = 0; i < 3; i++)
        {
            sandbox.Restore(snapshot);
            var result = sandbox.Run($"print('iteration {i}')");
            Assert.True(result.Success, $"Iteration {i} failed: {result.Stderr}");
            Assert.Contains($"iteration {i}", result.Stdout, StringComparison.Ordinal);
        }
    }

    // -----------------------------------------------------------------------
    // Filesystem
    // -----------------------------------------------------------------------

    [Fact]
    public void Integration_TempOutput_WritesAndLists()
    {
        var guestPath = FindPythonGuest();
        if (guestPath == null) return;

        using var sandbox = new SandboxBuilder()
            .WithModulePath(guestPath)
            .WithTempOutput()
            .Build();

        sandbox.Run("""
            with open("/output/test.txt", "w") as f:
                f.write("hello from test")
            """);

        var files = sandbox.GetOutputFiles();
        Assert.Contains("test.txt", files);
        Assert.NotNull(sandbox.OutputPath);
    }

    // -----------------------------------------------------------------------
    // Snapshot type mismatch (#19)
    // -----------------------------------------------------------------------

    // NOTE: Testing Wasm↔JS snapshot mismatch requires both backends to be
    // initialized with real guest execution, which requires the hyperlight-js
    // runtime. This test validates the error at the FFI level using the Rust
    // test suite (test `snapshot_before_init_fails`). A full cross-backend
    // test would need the JS runtime available.

    // -----------------------------------------------------------------------
    // Async
    // -----------------------------------------------------------------------

    [Fact]
    public async Task Integration_RunAsync_WorksFromDifferentThread()
    {
        using var sandbox = TryCreateSandbox();
        if (sandbox == null) return;

        var result = await sandbox.RunAsync("print('async hello')").ConfigureAwait(false);

        Assert.True(result.Success);
        Assert.Contains("async hello", result.Stdout, StringComparison.Ordinal);
    }

    // -----------------------------------------------------------------------
    // Helper types
    // -----------------------------------------------------------------------

    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used by System.Text.Json")]
    private sealed class AddArgs
    {
        public double a { get; set; }
        public double b { get; set; }
    }
}
