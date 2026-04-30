using System.Runtime.InteropServices;

namespace HyperlightSandbox.PInvoke;

/// <summary>
/// Matches the Rust <c>FFIResult</c> struct layout exactly.
///
/// On success: <c>is_success = true</c>, <c>error_code = 0</c>,
/// <c>value</c> may hold a pointer to an allocated string.
///
/// On failure: <c>is_success = false</c>, <c>error_code</c> classifies
/// the failure, <c>value</c> holds a UTF-8 error message string.
///
/// The caller is responsible for freeing <c>value</c> via
/// <see cref="SafeNativeMethods.hyperlight_sandbox_free_string"/>.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct FFIResult
{
    [MarshalAs(UnmanagedType.I1)]
    public bool is_success;

    public uint error_code;

    public IntPtr value;

    /// <summary>
    /// Returns <c>true</c> if the operation succeeded.
    /// </summary>
    public readonly bool IsSuccess() => is_success;

    /// <summary>
    /// Reads a UTF-8 string from <paramref name="ptr"/>, then frees the
    /// native memory. Returns <c>null</c> if <paramref name="ptr"/> is
    /// <see cref="IntPtr.Zero"/>.
    /// </summary>
    /// <remarks>
    /// This consumes ownership of the pointer — the Rust side allocated it,
    /// and we free it here. Do NOT call this twice on the same pointer.
    /// </remarks>
    public static string? StringFromPtr(IntPtr ptr)
    {
        if (ptr == IntPtr.Zero)
        {
            return null;
        }

        var str = Marshal.PtrToStringUTF8(ptr);
        // The Rust FFI layer expects the caller to free this string.
        SafeNativeMethods.hyperlight_sandbox_free_string(ptr);
        return str;
    }

    /// <summary>
    /// If the operation failed, throws an appropriate exception.
    /// If successful, does nothing.
    /// </summary>
    /// <remarks>
    /// Maps <see cref="FFIErrorCode"/> values to specific exception types
    /// for structured error handling in the API layer.
    /// </remarks>
    public void ThrowIfError()
    {
        if (is_success)
        {
            return;
        }

        var errorMessage = StringFromPtr(value) ?? "Unknown error from native layer.";
        var code = (FFIErrorCode)error_code;

        throw code switch
        {
            FFIErrorCode.Timeout =>
                new SandboxTimeoutException(errorMessage),
            FFIErrorCode.Poisoned =>
                new SandboxPoisonedException(errorMessage),
            FFIErrorCode.PermissionDenied =>
                new SandboxPermissionException(errorMessage),
            FFIErrorCode.InvalidArgument =>
                new ArgumentException(errorMessage),
            FFIErrorCode.IoError =>
                new System.IO.IOException(errorMessage),
            FFIErrorCode.GuestError =>
                new SandboxGuestException(errorMessage),
            _ =>
                new SandboxException($"Operation failed: {errorMessage}"),
        };
    }
}
