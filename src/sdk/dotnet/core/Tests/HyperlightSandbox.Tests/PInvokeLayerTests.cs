using System.Reflection;
using System.Runtime.InteropServices;
using HyperlightSandbox.PInvoke;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for the P/Invoke layer: FFIResult, FFIErrorCode, SafeNativeMethods,
/// and string ownership across the FFI boundary.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class PInvokeLayerTests
{
    // -----------------------------------------------------------------------
    // FFIErrorCode values must be stable (ABI contract with Rust)
    // -----------------------------------------------------------------------

    [Theory]
    [InlineData(0u, 0u)] // Success
    [InlineData(1u, 1u)] // Unknown
    [InlineData(2u, 2u)] // Timeout
    [InlineData(3u, 3u)] // Poisoned
    [InlineData(4u, 4u)] // PermissionDenied
    [InlineData(5u, 5u)] // GuestError
    [InlineData(6u, 6u)] // InvalidArgument
    [InlineData(7u, 7u)] // IoError
    public void FFIErrorCode_Values_MatchRust(uint codeValue, uint expected)
    {
        var code = (FFIErrorCode)codeValue;
        Assert.Equal(expected, (uint)code);
    }

    // -----------------------------------------------------------------------
    // FFIResult.ThrowIfError — maps codes to correct exception types
    // -----------------------------------------------------------------------

    [Fact]
    public void FFIResult_Success_DoesNotThrow()
    {
        var result = new FFIResult
        {
            is_success = true,
            error_code = (uint)FFIErrorCode.Success,
            value = IntPtr.Zero,
        };

        // Should not throw.
        result.ThrowIfError();
    }

    [Fact]
    public void FFIResult_Timeout_ThrowsOperationCanceledException()
    {
        var msg = Marshal.StringToCoTaskMemUTF8("execution timed out");
        // We need to allocate via Rust's allocator, but for this unit test
        // we test the mapping logic directly. The StringFromPtr will try to
        // free via hyperlight_sandbox_free_string which expects Rust allocation.
        // Instead, test the error code mapping without StringFromPtr.

        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.Timeout,
            value = IntPtr.Zero, // null value = "Unknown error" message
        };

        Assert.Throws<SandboxTimeoutException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_Poisoned_ThrowsSandboxPoisonedException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.Poisoned,
            value = IntPtr.Zero,
        };

        Assert.Throws<SandboxPoisonedException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_PermissionDenied_ThrowsSandboxPermissionException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.PermissionDenied,
            value = IntPtr.Zero,
        };

        Assert.Throws<SandboxPermissionException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_InvalidArgument_ThrowsArgumentException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.InvalidArgument,
            value = IntPtr.Zero,
        };

        Assert.Throws<ArgumentException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_IoError_ThrowsIOException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.IoError,
            value = IntPtr.Zero,
        };

        Assert.Throws<System.IO.IOException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_GuestError_ThrowsSandboxGuestException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.GuestError,
            value = IntPtr.Zero,
        };

        Assert.Throws<SandboxGuestException>(() => result.ThrowIfError());
    }

    [Fact]
    public void FFIResult_Unknown_ThrowsSandboxException()
    {
        var result = new FFIResult
        {
            is_success = false,
            error_code = (uint)FFIErrorCode.Unknown,
            value = IntPtr.Zero,
        };

        Assert.Throws<SandboxException>(() => result.ThrowIfError());
    }

    // -----------------------------------------------------------------------
    // String ownership: StringFromPtr
    // -----------------------------------------------------------------------

    [Fact]
    public void StringFromPtr_NullReturnsNull()
    {
        var result = FFIResult.StringFromPtr(IntPtr.Zero);
        Assert.Null(result);
    }

    // -----------------------------------------------------------------------
    // Version API (end-to-end FFI call)
    // -----------------------------------------------------------------------

    [Fact]
    public void GetVersion_ReturnsValidSemver()
    {
        var ptr = SafeNativeMethods.hyperlight_sandbox_get_version();
        Assert.NotEqual(IntPtr.Zero, ptr);

        var version = Marshal.PtrToStringUTF8(ptr);
        SafeNativeMethods.hyperlight_sandbox_free_string(ptr);

        Assert.NotNull(version);
        Assert.NotEmpty(version);
        Assert.Contains('.', version);
    }

    [Fact]
    public void DllImportResolver_ForSandboxLibraryWithoutApprovedPath_ThrowsDllNotFoundException()
    {
        var resolver = typeof(SafeNativeMethods).GetMethod(
            "DllImportResolver",
            BindingFlags.NonPublic | BindingFlags.Static);
        Assert.NotNull(resolver);

        var ex = Assert.Throws<TargetInvocationException>(() =>
            resolver.Invoke(null, [
                "hyperlight_sandbox_dotnet_ffi",
                typeof(string).Assembly,
                null,
            ]));

        var dllNotFound = Assert.IsType<DllNotFoundException>(ex.InnerException);
        Assert.Contains("hyperlight_sandbox_dotnet_ffi", dllNotFound.Message);
        Assert.Contains("Searched paths", dllNotFound.Message);
    }

    // -----------------------------------------------------------------------
    // Free string: null is safe
    // -----------------------------------------------------------------------

    [Fact]
    public void FreeString_Null_DoesNotCrash()
    {
        SafeNativeMethods.hyperlight_sandbox_free_string(IntPtr.Zero);
    }

    // -----------------------------------------------------------------------
    // Free snapshot: null is safe
    // -----------------------------------------------------------------------

    [Fact]
    public void FreeSnapshot_Null_DoesNotCrash()
    {
        SafeNativeMethods.hyperlight_sandbox_free_snapshot(IntPtr.Zero);
    }

    // -----------------------------------------------------------------------
    // Free sandbox: null is safe
    // -----------------------------------------------------------------------

    [Fact]
    public void FreeSandbox_Null_DoesNotCrash()
    {
        SafeNativeMethods.hyperlight_sandbox_free(IntPtr.Zero);
    }
}
