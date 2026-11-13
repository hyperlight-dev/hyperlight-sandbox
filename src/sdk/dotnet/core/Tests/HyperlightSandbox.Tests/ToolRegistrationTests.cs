using System.Collections.Concurrent;
using System.Diagnostics;
using System.Text.Json;
using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for tool registration API — both raw JSON and typed variants.
/// Tests registration validation, schema generation, error cases, and async dispatch.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class ToolRegistrationTests
{
    private static Sandbox CreateTestSandbox() =>
        new SandboxBuilder()
            .WithModulePath("/tmp/test-tools.wasm")
            .Build();

    // -----------------------------------------------------------------------
    // Raw JSON tool registration
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_RawJson_Succeeds()
    {
        using var sandbox = CreateTestSandbox();
        sandbox.RegisterTool("echo", (string json) => json);
    }

    [Fact]
    public void RegisterTool_RawJson_NullName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool(null!, (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_RawJson_EmptyName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool("", (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_RawJson_NullHandler_ThrowsArgumentNullException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.Throws<ArgumentNullException>(() =>
            sandbox.RegisterTool("test", (Func<string, string>)null!));
    }

    // -----------------------------------------------------------------------
    // Typed tool registration
    // -----------------------------------------------------------------------

    private sealed class AddArgs
    {
        public double a { get; set; }
        public double b { get; set; }
    }

    private sealed class AddResult
    {
        public double sum { get; set; }
    }

    [Fact]
    public void RegisterTool_Typed_Succeeds()
    {
        using var sandbox = CreateTestSandbox();
        sandbox.RegisterTool<AddArgs, AddResult>("add",
            args => new AddResult { sum = args.a + args.b });
    }

    [Fact]
    public void RegisterTool_Typed_NullName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>(null!,
                args => new AddResult { sum = 0 }));
    }

    [Fact]
    public void RegisterTool_Typed_NullHandler_ThrowsArgumentNullException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.Throws<ArgumentNullException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>("add", null!));
    }

    // -----------------------------------------------------------------------
    // Multiple tool registration
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_MultipleDifferentNames_Succeeds()
    {
        using var sandbox = CreateTestSandbox();

        sandbox.RegisterTool("tool1", (string json) => "{}");
        sandbox.RegisterTool("tool2", (string json) => "{}");
        sandbox.RegisterTool("tool3", (string json) => "{}");
        sandbox.RegisterTool<AddArgs, AddResult>("add",
            args => new AddResult { sum = args.a + args.b });
    }

    // -----------------------------------------------------------------------
    // Registration after dispose
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_AfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = CreateTestSandbox();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.RegisterTool("test", (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_Typed_AfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = CreateTestSandbox();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>("add",
                args => new AddResult { sum = 0 }));
    }

    [Theory]
    [InlineData(false, false)]
    [InlineData(true, false)]
    [InlineData(false, true)]
    [InlineData(true, true)]
    public async Task RegisterToolAsync_SynchronizationContext_DoesNotBlockRun(
        bool typed, bool throws)
    {
        var context = new QueuedSynchronizationContext();
        var entered = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var completed = new TaskCompletionSource<ExecutionResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        int handlerThread = 0;
        SynchronizationContext? handlerContext = context;
        SynchronizationContext? restoredContext = null;
        var thread = new Thread(() =>
        {
            SynchronizationContext.SetSynchronizationContext(context);
            try
            {
                using var sandbox = new SandboxBuilder()
                    .WithBackend(SandboxBackend.JavaScript)
                    .Build();

                async Task<int> HandleAsync()
                {
                    handlerThread = Environment.CurrentManagedThreadId;
                    handlerContext = SynchronizationContext.Current;
                    entered.TrySetResult(true);
                    await Task.Yield();
                    if (throws)
                    {
                        throw new InvalidOperationException("async tool failed");
                    }

                    return 42;
                }

                if (typed)
                {
                    sandbox.RegisterToolAsync<AddArgs, int>("async_tool", _ => HandleAsync());
                }
                else
                {
                    sandbox.RegisterToolAsync("async_tool", async _ =>
                        JsonSerializer.Serialize(await HandleAsync().ConfigureAwait(false)));
                }

                var result = sandbox.Run("""
                    try {
                        console.log(JSON.stringify(call_tool("async_tool", {a: 20, b: 22})));
                    } catch (error) {
                        console.log(String(error));
                    }
                    """);
                restoredContext = SynchronizationContext.Current;
                completed.TrySetResult(result);
            }
#pragma warning disable CA1031 // Forward failures from the dedicated thread to the test.
            catch (Exception error)
            {
                completed.TrySetException(error);
                entered.TrySetResult(true);
            }
#pragma warning restore CA1031
            finally
            {
                SynchronizationContext.SetSynchronizationContext(null);
            }
        })
        { IsBackground = true };

        thread.Start();
        try
        {
            await entered.Task.WaitAsync(TimeSpan.FromSeconds(30)).ConfigureAwait(false);
            var result = await completed.Task.WaitAsync(TimeSpan.FromSeconds(5))
                .ConfigureAwait(false);
            Assert.True(result.Success, result.Stderr);
            Assert.Contains(throws ? "async tool failed" : "42", result.Stdout, StringComparison.Ordinal);
            Assert.Equal(thread.ManagedThreadId, handlerThread);
            Assert.Null(handlerContext);
            Assert.Same(context, restoredContext);
        }
        finally
        {
            // Release captured continuations after a failed assertion so the test leaves no blocked worker.
            var elapsed = Stopwatch.StartNew();
            while (thread.IsAlive && elapsed.Elapsed < TimeSpan.FromSeconds(10))
            {
                context.Drain();
                thread.Join(10);
            }

            Assert.False(thread.IsAlive, "The sandbox thread did not stop during test cleanup.");
        }
    }

    private sealed class QueuedSynchronizationContext : SynchronizationContext
    {
        private readonly ConcurrentQueue<(SendOrPostCallback Callback, object? State)> _callbacks = new();

        public override void Post(SendOrPostCallback d, object? state) => _callbacks.Enqueue((d, state));

        public void Drain()
        {
            while (_callbacks.TryDequeue(out var callback))
            {
                callback.Callback(callback.State);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Schema generation (ToolSchemaBuilder)
    // -----------------------------------------------------------------------

    [Fact]
    public void ToolSchemaBuilder_NumericTypes_MapToNumber()
    {
        var schema = ToolSchemaBuilder.BuildSchema<NumericArgs>();
        Assert.Contains("\"Number\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_StringType_MapsToString()
    {
        var schema = ToolSchemaBuilder.BuildSchema<StringArgs>();
        Assert.Contains("\"String\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_BoolType_MapsToBoolean()
    {
        var schema = ToolSchemaBuilder.BuildSchema<BoolArgs>();
        Assert.Contains("\"Boolean\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_ComplexType_MapsToObject()
    {
        var schema = ToolSchemaBuilder.BuildSchema<ComplexArgs>();
        Assert.Contains("\"Object\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_ArrayType_MapsToArray()
    {
        var schema = ToolSchemaBuilder.BuildSchema<ArrayArgs>();
        Assert.Contains("\"Array\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_AllPropertiesRequired()
    {
        var schema = ToolSchemaBuilder.BuildSchema<AddArgs>();
        Assert.Contains("\"a\"", schema, StringComparison.Ordinal);
        Assert.Contains("\"b\"", schema, StringComparison.Ordinal);
        Assert.Contains("required", schema, StringComparison.Ordinal);
    }

    // Schema test helper types — instantiated by System.Text.Json reflection
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class NumericArgs { public int Value { get; set; } }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class StringArgs { public string Name { get; set; } = ""; }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class BoolArgs { public bool Flag { get; set; } }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class ComplexArgs { public AddArgs Nested { get; set; } = new(); }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class ArrayArgs { public int[] Items { get; set; } = []; }
}
