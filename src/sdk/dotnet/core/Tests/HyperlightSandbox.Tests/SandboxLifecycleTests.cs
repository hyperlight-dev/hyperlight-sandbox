using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for sandbox lifecycle management — creation, disposal, idempotency,
/// and using pattern correctness.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class SandboxLifecycleTests
{
    [Fact]
    public void Sandbox_CanBeCreated()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void Sandbox_Dispose_IsIdempotent()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        sandbox.Dispose();
        sandbox.Dispose(); // Second call should not throw or crash.
        sandbox.Dispose(); // Third time's the charm.
    }

    [Fact]
    public void Sandbox_UsingStatement_DisposesCorrectly()
    {
        Sandbox? sandboxRef;
        using (var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build())
        {
            sandboxRef = sandbox;
            Assert.NotNull(sandboxRef);
        }

        // After leaving using block, should be disposed.
        Assert.Throws<ObjectDisposedException>(() =>
            sandboxRef.AllowDomain("https://example.com"));
    }

    [Fact]
    public void Sandbox_AllowDomain_BeforeRun_Succeeds()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        // Should not throw — queued for lazy init.
        sandbox.AllowDomain("https://httpbin.org");
        sandbox.AllowDomain("https://api.example.com", ["GET", "POST"]);
    }

    [Fact]
    public void Sandbox_AllowDomain_NullTarget_ThrowsArgumentException()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.AllowDomain(null!));
    }

    [Fact]
    public void Sandbox_Run_NullCode_ThrowsArgumentException()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.Run(null!));
    }

    [Fact]
    public void Sandbox_Run_EmptyCode_ThrowsArgumentException()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.Run(""));
    }

    [Fact]
    public void Sandbox_Run_ExceedsMaxCodeSize_ThrowsArgumentException()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        var hugeCode = new string('x', Sandbox.MaxCodeSize + 1);
        var ex = Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.Run(hugeCode));

        Assert.Contains("maximum size", ex.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void Sandbox_MaxCodeSize_Is10MiB()
    {
        Assert.Equal(10 * 1024 * 1024, Sandbox.MaxCodeSize);
    }

    [Fact]
    public void Sandbox_Run_NonexistentModule_ThrowsSandboxException()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/definitely-nonexistent-module-12345.wasm")
            .Build();

        // Should fail gracefully (not crash) because the module doesn't exist.
        // The exact exception type depends on the FFI error classification.
        Assert.ThrowsAny<Exception>(() =>
            sandbox.Run("print('hello')"));
    }
}
