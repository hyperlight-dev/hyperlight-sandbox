using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for the JavaScript backend (SandboxBackend.JavaScript).
/// Validates that the builder, FFI create, and lifecycle all work
/// correctly for the JS backend path.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class JavaScriptBackendTests
{
    // -----------------------------------------------------------------------
    // Builder validation
    // -----------------------------------------------------------------------

    [Fact]
    public void WithBackend_JavaScript_NoModulePath_Succeeds()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithBackend_JavaScript_WithModulePath_ThrowsInvalidOperationException()
    {
        var builder = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .WithModulePath("/tmp/should-not-be-set.wasm");

        Assert.Throws<InvalidOperationException>(() => builder.Build());
    }

    [Fact]
    public void WithBackend_Wasm_WithoutModulePath_ThrowsInvalidOperationException()
    {
        var builder = new SandboxBuilder()
            .WithBackend(SandboxBackend.Wasm);

        Assert.Throws<InvalidOperationException>(() => builder.Build());
    }

    [Fact]
    public void WithBackend_Default_IsWasm()
    {
        // Default backend requires module path (Wasm)
        var builder = new SandboxBuilder();
        Assert.Equal(SandboxBackend.Wasm, builder.Backend);
        Assert.Throws<InvalidOperationException>(() => builder.Build());
    }

    [Fact]
    public void WithBackend_UpdatesBackendProperty()
    {
        var builder = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript);

        Assert.Equal(SandboxBackend.JavaScript, builder.Backend);
    }

    // -----------------------------------------------------------------------
    // Lifecycle
    // -----------------------------------------------------------------------

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task JavaScript_RunAsync_CancellationDuringExecution_StopsGuestAsync(bool delayedCancellation)
    {
        using var cancellation = new CancellationTokenSource();
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();
        sandbox.RegisterTool("cancel_run", (string _) =>
        {
            if (delayedCancellation)
            {
                cancellation.CancelAfter(TimeSpan.FromMilliseconds(100));
            }
            else
            {
                cancellation.Cancel();
            }

            return "{}";
        });
        Assert.True((await sandbox.RunAsync("console.log('ready');").ConfigureAwait(false)).Success);
        using var snapshot = await sandbox.SnapshotAsync().ConfigureAwait(false);
        var elapsed = System.Diagnostics.Stopwatch.StartNew();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => sandbox.RunAsync("""
            call_tool('cancel_run');
            const end = Date.now() + 5000;
            while (Date.now() < end) {}
            console.log('finished');
            """, cancellation.Token)).ConfigureAwait(false);

        Assert.True(elapsed.Elapsed < TimeSpan.FromSeconds(2), "Guest execution was not interrupted promptly.");
        await sandbox.RestoreAsync(snapshot).ConfigureAwait(false);
        using var recovery = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var result = await sandbox.RunAsync("console.log('recovered');", recovery.Token)
            .WaitAsync(TimeSpan.FromSeconds(10)).ConfigureAwait(false);
        Assert.True(result.Success, result.Stderr);
        Assert.Contains("recovered", result.Stdout, StringComparison.Ordinal);
    }

    [Fact]
    public void JavaScript_Run_PreCanceledTokenDoesNotExecuteGuest()
    {
        using var cancellation = new CancellationTokenSource();
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();
        var invoked = false;
        sandbox.RegisterTool("mark_run", (string _) =>
        {
            invoked = true;
            return "{}";
        });
        cancellation.Cancel();

        Assert.ThrowsAny<OperationCanceledException>(
            () => sandbox.Run("call_tool('mark_run');", cancellation.Token));
        Assert.False(invoked);
        using var recovery = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        Assert.True(sandbox.Run("console.log('recovered');", recovery.Token).Success);
    }

    [Fact]
    public void JavaScript_Run_CompletedCancellationDoesNotInterruptNextRun()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();
        for (int i = 0; i < 5; i++)
        {
            using var completed = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            Assert.True(sandbox.Run("console.log('first');", completed.Token).Success);
            completed.Cancel();

            using var next = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            var result = sandbox.Run("console.log('next');", next.Token);
            Assert.True(result.Success, result.Stderr);
            Assert.Contains("next", result.Stdout, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void JavaScript_CreateAndDispose_NoLeak()
    {
        for (int i = 0; i < 10; i++)
        {
            using var sandbox = new SandboxBuilder()
                .WithBackend(SandboxBackend.JavaScript)
                .Build();
        }
    }

    [Fact]
    public void JavaScript_Dispose_IsIdempotent()
    {
        var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();

        sandbox.Dispose();
        sandbox.Dispose();
        sandbox.Dispose();
    }

    [Fact]
    public void JavaScript_UseAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.Run("console.log('hello');"));
    }

    // -----------------------------------------------------------------------
    // Configuration
    // -----------------------------------------------------------------------

    [Fact]
    public void JavaScript_AllowDomain_QueuesBeforeInit()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();

        // Should not throw — queued for lazy init
        sandbox.AllowDomain("https://httpbin.org");
    }

    [Fact]
    public void JavaScript_RegisterTool_Succeeds()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .Build();

        sandbox.RegisterTool("echo", (string json) => json);
    }

    [Fact]
    public void JavaScript_WithTempOutput_Succeeds()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .WithTempOutput()
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void JavaScript_WithInputDir_Succeeds()
    {
        using var sandbox = new SandboxBuilder()
            .WithBackend(SandboxBackend.JavaScript)
            .WithInputDir("/tmp")
            .Build();

        Assert.NotNull(sandbox);
    }

    // -----------------------------------------------------------------------
    // Backend enum values
    // -----------------------------------------------------------------------

    [Fact]
    public void SandboxBackend_Values_AreCorrect()
    {
        Assert.Equal(0, (int)SandboxBackend.Wasm);
        Assert.Equal(1, (int)SandboxBackend.JavaScript);
    }
}
