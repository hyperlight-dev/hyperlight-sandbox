using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for <see cref="SandboxBuilder"/> configuration and validation.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class SandboxBuilderTests
{
    [Fact]
    public void Build_WithModulePath_CreatesSandbox()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void Build_WithoutModulePath_ThrowsInvalidOperationException()
    {
        var builder = new SandboxBuilder();
        Assert.Throws<InvalidOperationException>(() => builder.Build());
    }

    [Fact]
    public void WithModulePath_NullOrEmpty_ThrowsArgumentException()
    {
        var builder = new SandboxBuilder();
        Assert.ThrowsAny<ArgumentException>(() => builder.WithModulePath(null!));
        Assert.ThrowsAny<ArgumentException>(() => builder.WithModulePath(""));
        Assert.ThrowsAny<ArgumentException>(() => builder.WithModulePath("  "));
    }

    [Fact]
    public void WithHeapSize_StringFormat_Parses()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithHeapSize("50Mi")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithHeapSize_ByteValue_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithHeapSize(50UL * 1024 * 1024)
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithStackSize_StringFormat_Parses()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithStackSize("10Mi")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithStackSize_ByteValue_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithStackSize(10UL * 1024 * 1024)
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithInputDir_ValidPath_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithInputDir("/tmp/input")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithInputDir_NullOrEmpty_ThrowsArgumentException()
    {
        var builder = new SandboxBuilder();
        Assert.ThrowsAny<ArgumentException>(() => builder.WithInputDir(null!));
        Assert.ThrowsAny<ArgumentException>(() => builder.WithInputDir(""));
    }

    [Fact]
    public void WithOutputDir_ValidPath_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithOutputDir("/tmp/output")
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithTempOutput_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithTempOutput()
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void ChainedConfiguration_AllOptions_Works()
    {
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm")
            .WithHeapSize("100Mi")
            .WithStackSize("50Mi")
            .WithInputDir("/tmp/input")
            .WithTempOutput()
            .Build();

        Assert.NotNull(sandbox);
    }

    [Fact]
    public void Builder_CanBeReused()
    {
        var builder = new SandboxBuilder()
            .WithModulePath("/tmp/test.wasm");

        using var sandbox1 = builder.Build();
        using var sandbox2 = builder.Build();

        Assert.NotNull(sandbox1);
        Assert.NotNull(sandbox2);
    }
}
