namespace HyperlightSandbox.PInvoke;

/// <summary>
/// Error classification codes returned by the FFI layer.
/// These map 1:1 to the Rust <c>FFIErrorCode</c> enum in
/// <c>src/sdk/dotnet/ffi/src/lib.rs</c>.
///
/// Used by <see cref="FFIResult.ThrowIfError"/> to map native errors
/// to specific .NET exception types.
/// </summary>
internal enum FFIErrorCode : uint
{
    /// <summary>No error.</summary>
    Success = 0,

    /// <summary>Unclassified error.</summary>
    Unknown = 1,

    /// <summary>Execution exceeded a time limit.</summary>
    Timeout = 2,

    /// <summary>Sandbox state is poisoned (mutex or guest crash).</summary>
    Poisoned = 3,

    /// <summary>Network permission denied.</summary>
    PermissionDenied = 4,

    /// <summary>Guest code raised an error.</summary>
    GuestError = 5,

    /// <summary>Invalid argument passed to FFI function.</summary>
    InvalidArgument = 6,

    /// <summary>Filesystem I/O error.</summary>
    IoError = 7,
}
