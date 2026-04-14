using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for SafeHandle lifecycle, ownership transfers, GC interactions,
/// and ensuring no double-frees or use-after-free across the Rust ↔ .NET boundary.
///
/// These are the MOST CRITICAL tests in the entire SDK — they validate
/// memory safety at the FFI boundary.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class OwnershipTransferTests
{
    // -----------------------------------------------------------------------
    // 1. SafeHandle lifecycle: Create → use → Dispose → no double-free
    // -----------------------------------------------------------------------

    [Fact]
    public void SandboxHandle_Create_Dispose_NoCrash()
    {
        // A sandbox with a nonexistent module path — we're testing handle
        // lifecycle, not execution. The create FFI call succeeds (lazy init).
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-lifecycle.wasm")
            .Build();

        // Dispose should free the native handle exactly once.
        sandbox.Dispose();

        // Second dispose should be a no-op (idempotent).
        sandbox.Dispose();
    }

    [Fact]
    public void SandboxHandle_UsingPattern_NoLeak()
    {
        // The using pattern should free the handle correctly.
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-using.wasm")
            .Build();
    }

    // -----------------------------------------------------------------------
    // 2. SafeHandle finalizer: abandon without Dispose → GC cleans up
    // -----------------------------------------------------------------------

    [Fact]
    public void SandboxHandle_Abandoned_FinalizerFreesCorrectly()
    {
        // Create a sandbox in a separate method so it goes out of scope.
        CreateAndAbandonSandbox();

        // Force GC to run finalizers. If the finalizer double-frees or
        // accesses invalid memory, this will crash (SIGSEGV).
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);
        GC.WaitForPendingFinalizers();
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);

        // If we get here, the finalizer ran without crashing.
    }

    private static void CreateAndAbandonSandbox()
    {
        // Intentionally NOT disposing — let the finalizer handle it.
        _ = new SandboxBuilder()
            .WithModulePath("/tmp/test-finalizer.wasm")
            .Build();
    }

    // -----------------------------------------------------------------------
    // 3. Dispose + finalizer race (Interlocked.Exchange prevents double-free)
    // -----------------------------------------------------------------------

    [Fact]
    public void SandboxHandle_DisposeAndFinalizerRace_NoDoubleFree()
    {
        for (int i = 0; i < 100; i++)
        {
            var sandbox = new SandboxBuilder()
                .WithModulePath("/tmp/test-race.wasm")
                .Build();

            // Dispose on this thread...
            sandbox.Dispose();

            // ...while GC might finalize on another thread.
            GC.Collect();
            GC.WaitForPendingFinalizers();
        }

        // 100 iterations without crash = Interlocked.Exchange works.
    }

    // -----------------------------------------------------------------------
    // 4. Tool callback GCHandle pinning — delegates survive GC
    // -----------------------------------------------------------------------

    [Fact]
    public void ToolCallback_PinnedDuringLifetime_SurvivesGC()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-pin.wasm")
            .Build();

        // Register a tool — the delegate must be pinned.
        sandbox.RegisterTool("test_tool", (string json) =>
        {
            return """{"result": "ok"}""";
        });

        // Force GC — if the delegate isn't pinned, the fn pointer becomes
        // dangling and calling it from Rust would SIGSEGV.
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);
        GC.WaitForPendingFinalizers();
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);

        // The sandbox is still alive, and the pinned delegate should survive.
        // We can't invoke the callback without a real module, but we verify
        // no crash from GC.

        sandbox.Dispose(); // This should free the GCHandle.
    }

    [Fact]
    public void ToolCallback_MultiplePinned_AllFreedOnDispose()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-multi-pin.wasm")
            .Build();

        // Register multiple tools.
        for (int i = 0; i < 10; i++)
        {
            var toolNum = i;
            sandbox.RegisterTool($"tool_{toolNum}", (string json) =>
            {
                return $"{{\"tool\": {toolNum}}}";
            });
        }

        // Force GC aggressively.
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);
        GC.WaitForPendingFinalizers();

        // All 10 pinned delegates should survive.
        // Dispose should free all 10 GCHandles.
        sandbox.Dispose();

        // Another GC should not crash (handles already freed).
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true);
        GC.WaitForPendingFinalizers();
    }

    // -----------------------------------------------------------------------
    // 5. Disposed object operations → ObjectDisposedException (not SIGSEGV)
    // -----------------------------------------------------------------------

    [Fact]
    public void Sandbox_RunAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-run.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.Run("print('hello')"));
    }

    [Fact]
    public void Sandbox_RegisterToolAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-tool.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.RegisterTool("test", (string json) => "{}"));
    }

    [Fact]
    public void Sandbox_AllowDomainAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-domain.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.AllowDomain("https://example.com"));
    }

    [Fact]
    public void Sandbox_SnapshotAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-snapshot.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.Snapshot());
    }

    [Fact]
    public void Sandbox_GetOutputFilesAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-files.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.GetOutputFiles());
    }

    [Fact]
    public void Sandbox_OutputPathAfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-disposed-path.wasm")
            .Build();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            _ = sandbox.OutputPath);
    }

    // -----------------------------------------------------------------------
    // 6. Concurrent GC stress — operations with GC.Collect in background
    // -----------------------------------------------------------------------

    [Fact]
    public void ConcurrentGCStress_NoDoubleFreeOrSIGSEGV()
    {
        // Run 50 create/dispose cycles while GC runs aggressively.
        using var cts = new CancellationTokenSource();

        // Background GC pressure thread.
        var gcTask = Task.Run(() =>
        {
            while (!cts.Token.IsCancellationRequested)
            {
                GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: false);
                Thread.Sleep(1); // Yield to other threads.
            }
        });

        try
        {
            for (int i = 0; i < 50; i++)
            {
                var sandbox = new SandboxBuilder()
                    .WithModulePath($"/tmp/test-gc-stress-{i}.wasm")
                    .Build();

                sandbox.RegisterTool("stress_tool", (string json) => "{}");
                sandbox.AllowDomain("https://example.com");

                sandbox.Dispose();
            }
        }
        finally
        {
            cts.Cancel();
            gcTask.Wait();
        }
    }

    // -----------------------------------------------------------------------
    // 7. Memory leak detection — repeated create/free loops
    // -----------------------------------------------------------------------

    [Fact]
    public void MemoryLeak_RepeatedCreateDispose_NoGrowth()
    {
        // Warm up.
        for (int i = 0; i < 10; i++)
        {
            using var s = new SandboxBuilder()
                .WithModulePath("/tmp/test-warmup.wasm")
                .Build();
        }

        ForceFullGC();
        var memBefore = GC.GetTotalMemory(forceFullCollection: false);

        const int iterations = 200;
        for (int i = 0; i < iterations; i++)
        {
            using var sandbox = new SandboxBuilder()
                .WithModulePath("/tmp/test-leak.wasm")
                .Build();

            sandbox.RegisterTool("leak_tool", (string json) => "{}");
        }

        ForceFullGC();
        var memAfter = GC.GetTotalMemory(forceFullCollection: false);

        var growth = memAfter - memBefore;
        // Native sandbox handles + managed wrappers create some overhead per
        // iteration. We're checking for unbounded growth, not zero growth.
        // Anything under 500KB per iteration average is acceptable.
        var maxGrowth = (long)iterations * 500_000;
        Assert.True(growth < maxGrowth,
            $"LEAK DETECTED: Memory grew by {growth:N0} bytes over {iterations} iterations " +
            $"(max allowed: {maxGrowth:N0})");
    }

    [Fact]
    public void MemoryLeak_AbandonedSandboxes_FinalizerCleansUp()
    {
        ForceFullGC();
        var memBefore = GC.GetTotalMemory(forceFullCollection: false);

        for (int i = 0; i < 100; i++)
        {
            // Intentionally NOT disposing — relying on finalizer.
            _ = new SandboxBuilder()
                .WithModulePath("/tmp/test-abandon-leak.wasm")
                .Build();
        }

        // Force GC to run finalizers (multiple passes for generational GC).
        ForceFullGC();
        ForceFullGC();

        var memAfter = GC.GetTotalMemory(forceFullCollection: false);
        var growth = memAfter - memBefore;

        Assert.True(growth < 200_000,
            $"LEAK DETECTED: Abandoned sandboxes leaked {growth:N0} bytes");
    }

    // -----------------------------------------------------------------------
    // 8. Cross-thread access is safe (Send semantics — lock serializes)
    // -----------------------------------------------------------------------

    [Fact]
    public async Task CrossThreadAccess_WithLock_IsSerializedSafely()
    {
        var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test-thread.wasm")
            .Build();

        // Access from a different thread should work (Send, not Sync).
        // The internal lock prevents concurrent access.
        await Task.Run(() => sandbox.AllowDomain("https://example.com")).ConfigureAwait(false);

        // Back on original thread — should also work.
        sandbox.AllowDomain("https://another.com");

        sandbox.Dispose();
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    private static void ForceFullGC()
    {
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true, compacting: true);
        GC.WaitForPendingFinalizers();
        GC.Collect(GC.MaxGeneration, GCCollectionMode.Forced, blocking: true, compacting: true);
    }
}
