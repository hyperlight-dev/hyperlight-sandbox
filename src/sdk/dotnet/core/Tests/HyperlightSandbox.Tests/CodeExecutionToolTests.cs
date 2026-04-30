using HyperlightSandbox.Api;
using HyperlightSandbox.Extensions.AI;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for <see cref="CodeExecutionTool"/> — the AI agent integration wrapper.
/// Tests initialization, dispose, snapshot/restore-per-call, and AIFunction creation.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class CodeExecutionToolTests
{
    private static SandboxBuilder TestBuilder() =>
        new SandboxBuilder().WithModulePath("/tmp/code-exec-tool-test.wasm");

    // -----------------------------------------------------------------------
    // Construction and disposal
    // -----------------------------------------------------------------------

    [Fact]
    public void Create_WithBuilder_Succeeds()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        Assert.NotNull(tool);
    }

    [Fact]
    public void Create_WithJavaScriptBackend_Succeeds()
    {
        using var tool = new CodeExecutionTool(
            new SandboxBuilder().WithBackend(SandboxBackend.JavaScript));

        Assert.NotNull(tool);
    }

    [Fact]
    public void Create_NullBuilder_ThrowsArgumentNullException()
    {
        Assert.Throws<ArgumentNullException>(() =>
            new CodeExecutionTool(null!));
    }

    [Fact]
    public void Dispose_IsIdempotent()
    {
        var tool = new CodeExecutionTool(TestBuilder());
        tool.Dispose();
        tool.Dispose();
        tool.Dispose();
    }

    [Fact]
    public void Execute_AfterDispose_ThrowsObjectDisposedException()
    {
        var tool = new CodeExecutionTool(TestBuilder());
        tool.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            tool.Execute("print('hello')"));
    }

    [Theory]
    [InlineData(SandboxBackend.Wasm, "None")]
    [InlineData(SandboxBackend.JavaScript, "void 0;")]
    public void InitializationCodeFor_UsesBackendNoOp(
        SandboxBackend backend,
        string expectedCode)
    {
        Assert.Equal(expectedCode, CodeExecutionTool.InitializationCodeFor(backend));
    }

    // -----------------------------------------------------------------------
    // Tool registration
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_RawJson_Succeeds()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        tool.RegisterTool("echo", (string json) => json);
    }

    [Fact]
    public void RegisterTool_Typed_Succeeds()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        tool.RegisterTool<TestArgs, double>("add", args => args.a + args.b);
    }

    [Fact]
    public void RegisterTool_AfterDispose_ThrowsObjectDisposedException()
    {
        var tool = new CodeExecutionTool(TestBuilder());
        tool.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            tool.RegisterTool("test", (string json) => "{}"));
    }

    // -----------------------------------------------------------------------
    // AllowDomain
    // -----------------------------------------------------------------------

    [Fact]
    public void AllowDomain_BeforeExecute_Succeeds()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        tool.AllowDomain("https://httpbin.org");
        tool.AllowDomain("https://example.com", ["GET", "POST"]);
    }

    [Fact]
    public void AllowDomain_AfterDispose_ThrowsObjectDisposedException()
    {
        var tool = new CodeExecutionTool(TestBuilder());
        tool.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            tool.AllowDomain("https://example.com"));
    }

    // -----------------------------------------------------------------------
    // AsAIFunction
    // -----------------------------------------------------------------------

    [Fact]
    public void AsAIFunction_DefaultName_ReturnsExecuteCode()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        var fn = tool.AsAIFunction();

        Assert.Equal("execute_code", fn.Name);
        Assert.NotNull(fn.Description);
        Assert.NotEmpty(fn.Description);
    }

    [Fact]
    public void AsAIFunction_CustomName_UsesIt()
    {
        using var tool = new CodeExecutionTool(TestBuilder());
        var fn = tool.AsAIFunction(name: "run_code", description: "Custom desc");

        Assert.Equal("run_code", fn.Name);
        Assert.Equal("Custom desc", fn.Description);
    }

    [Fact]
    public void AsAIFunction_AfterDispose_ThrowsObjectDisposedException()
    {
        var tool = new CodeExecutionTool(TestBuilder());
        tool.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            tool.AsAIFunction());
    }

    // -----------------------------------------------------------------------
    // Thread safety
    // -----------------------------------------------------------------------

    [Fact]
    public void ConcurrentAccess_DoesNotCrash()

    {
        using var tool = new CodeExecutionTool(TestBuilder());
        tool.RegisterTool("test", (string json) => "{}");

        // Multiple threads registering tools concurrently
        var tasks = Enumerable.Range(0, 5).Select(i =>
            Task.Run(() => tool.AllowDomain($"https://example{i}.com"))
        ).ToArray();

        Task.WaitAll(tasks);
    }

    // -----------------------------------------------------------------------
    // Helper types
    // -----------------------------------------------------------------------

    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter")]
    private sealed class TestArgs
    {
        public double a { get; set; }
        public double b { get; set; }
    }
}
