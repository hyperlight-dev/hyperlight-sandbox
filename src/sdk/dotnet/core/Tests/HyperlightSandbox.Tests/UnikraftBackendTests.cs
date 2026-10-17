using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Builder validation for the Unikraft backend (SandboxBackend.Unikraft).
/// None of these boot a guest: the rootfs is opened on the first run.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class UnikraftBackendTests
{
    private static SandboxBuilder Unikraft() => new SandboxBuilder()
        .WithBackend(SandboxBackend.Unikraft)
        .WithModulePath("/tmp/test-rootfs.cpio");

    [Fact]
    public void WithScratchSize_Builds()
    {
        using var sandbox = Unikraft().WithScratchSize("512Mi").Build();
        Assert.NotNull(sandbox);
    }

    [Fact]
    public void WithScratchSize_BelowOneMiB_Throws()
    {
        Assert.Throws<ArgumentException>(() => Unikraft().WithScratchSize(512 * 1024).Build());
    }

    [Fact]
    public void WithHeapSize_Throws()
    {
        var error = Assert.Throws<InvalidOperationException>(() => Unikraft().WithHeapSize("64Mi").Build());
        Assert.Contains("WithScratchSize", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void WithStackSize_Throws()
    {
        Assert.Throws<InvalidOperationException>(() => Unikraft().WithStackSize("1Mi").Build());
    }

    [Fact]
    public void WithScratchSize_OnOtherBackends_Throws()
    {
        Assert.Throws<InvalidOperationException>(() =>
            new SandboxBuilder().WithBackend(SandboxBackend.JavaScript).WithScratchSize("64Mi").Build());
    }

    [Fact]
    public void WithModulePath_IsRequired()
    {
        Assert.Throws<InvalidOperationException>(() =>
            new SandboxBuilder().WithBackend(SandboxBackend.Unikraft).Build());
    }
}
