using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.PackageTests;

/// <summary>
/// Validates that the NuGet packages can be installed and used.
/// These are smoke tests to verify correct packaging before publishing.
///
/// Run via: just dotnet package-test
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class PackageInstallationTests
{
    /// <summary>
    /// Verifies that the package can be installed, a sandbox created,
    /// and basic operations work (API types resolve, FFI loads).
    /// </summary>
    [Fact]
    public void Api_CanCreateSandboxBuilder()
    {
        // If this compiles and runs, the package is correctly installed
        // and the API types are accessible.
        var builder = new SandboxBuilder();
        Assert.NotNull(builder);
    }

    [Fact]
    public void Api_SandboxBackendEnum_HasExpectedValues()
    {
        Assert.Equal(0, (int)SandboxBackend.Wasm);
        Assert.Equal(1, (int)SandboxBackend.JavaScript);
    }

    [Fact]
    public void Api_ExecutionResult_RecordWorks()
    {
        var result = new ExecutionResult("hello\n", "", 0);
        Assert.True(result.Success);
        Assert.Equal("hello\n", result.Stdout);
        Assert.Equal(0, result.ExitCode);
    }

    [Fact]
    public void Api_SandboxBuilder_WithModulePath_RequiredForWasm()
    {
        var builder = new SandboxBuilder();
        Assert.Throws<InvalidOperationException>(() => builder.Build());
    }

    [Fact]
    public void Api_ExceptionTypes_AreAccessible()
    {
        // Verify custom exception types are public and usable.
        var ex1 = new SandboxException("test");
        var ex2 = new SandboxTimeoutException("test");
        var ex3 = new SandboxPoisonedException("test");
        var ex4 = new SandboxPermissionException("test");
        var ex5 = new SandboxGuestException("test");

        Assert.IsAssignableFrom<SandboxException>(ex2);
        Assert.IsAssignableFrom<SandboxException>(ex3);
        Assert.IsAssignableFrom<SandboxException>(ex4);
        Assert.IsAssignableFrom<SandboxException>(ex5);
        Assert.IsAssignableFrom<Exception>(ex1);
    }

    /// <summary>
    /// Verifies that the native FFI library loads correctly.
    /// This test creates a sandbox with a nonexistent module — we're testing
    /// that the P/Invoke layer initializes, not that execution works.
    /// </summary>
    [Fact]
    public void PInvoke_NativeLibrary_LoadsAndCreatesHandle()
    {
        // This will create the native sandbox state (lazy init,
        // so no module load until Run is called).
        using var sandbox = new SandboxBuilder()
            .WithModulePath("/tmp/package-test-nonexistent.wasm")
            .Build();

        Assert.NotNull(sandbox);
    }
}
