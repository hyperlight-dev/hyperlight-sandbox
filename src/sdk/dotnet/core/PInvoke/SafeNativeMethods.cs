using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace HyperlightSandbox.PInvoke;

#pragma warning disable CA5392 // Use DefaultDllImportSearchPaths attribute for P/Invokes
// Justification: We use LibraryImport with a custom NativeLibrary.SetDllImportResolver.
// The resolver loads the library from the assembly directory or runtimes/<rid>/native/,
// which is safer than default search paths. CA5392 is not applicable to custom resolvers.

/// <summary>
/// P/Invoke declarations for the <c>hyperlight_sandbox_ffi</c> native library.
///
/// Every function here maps 1:1 to an <c>extern "C"</c> function in
/// <c>src/sdk/dotnet/ffi/src/lib.rs</c>.
/// </summary>
/// <remarks>
/// <para>
/// <b>String ownership</b>: All string pointers returned in
/// <see cref="FFIResult.value"/> are allocated by Rust and must be freed
/// via <see cref="hyperlight_sandbox_free_string"/>. Use
/// <see cref="FFIResult.StringFromPtr"/> which handles this automatically.
/// </para>
/// <para>
/// <b>Handle ownership</b>: Handles returned by <c>_create</c> /
/// <c>_snapshot</c> are heap-allocated Rust <c>Box</c> values. They must
/// be freed exactly once via the corresponding <c>_free</c> function.
/// The <see cref="SandboxSafeHandle"/> and <see cref="SnapshotSafeHandle"/>
/// classes handle this automatically via <see cref="SafeHandle"/>.
/// </para>
/// </remarks>
internal static partial class SafeNativeMethods
{
    private const string LibName = "hyperlight_sandbox_dotnet_ffi";

    static SafeNativeMethods()
    {
        NativeLibrary.SetDllImportResolver(
            typeof(SafeNativeMethods).Assembly,
            DllImportResolver);
    }

    /// <summary>
    /// Resolves the native library path for the current platform.
    /// Checks RID-specific paths first (for NuGet package layout),
    /// then falls back to the assembly directory.
    /// </summary>
    private static IntPtr DllImportResolver(
        string libraryName,
        Assembly assembly,
        DllImportSearchPath? searchPath)
    {
        if (libraryName != LibName)
        {
            return IntPtr.Zero;
        }

        string assemblyDirectory = Path.GetDirectoryName(assembly.Location) ?? string.Empty;

        // Platform-specific library filename
        string platformLibraryName = OperatingSystem.IsWindows()
            ? $"{libraryName}.dll"
            : $"lib{libraryName}.so";

        // Check RID-specific path (NuGet package layout: runtimes/<rid>/native/)
        string rid = OperatingSystem.IsWindows() ? "win-x64" : "linux-x64";
        string runtimePath = Path.Join(
            assemblyDirectory, "runtimes", rid, "native", platformLibraryName);

        if (File.Exists(runtimePath))
        {
            return NativeLibrary.Load(runtimePath);
        }

        // Check assembly directory directly (local development)
        string localPath = Path.Join(assemblyDirectory, platformLibraryName);
        if (File.Exists(localPath))
        {
            return NativeLibrary.Load(localPath);
        }

        // Check Rust target directory (development builds only)
        // Guarded to prevent loading from unexpected locations in production.
#if DEBUG
        string? dir = assemblyDirectory;
        while (dir != null)
        {
            string cargoTarget = Path.Join(dir, "target", "debug", platformLibraryName);
            if (File.Exists(cargoTarget))
            {
                return NativeLibrary.Load(cargoTarget);
            }

            string cargoTargetRelease = Path.Join(dir, "target", "release", platformLibraryName);
            if (File.Exists(cargoTargetRelease))
            {
                return NativeLibrary.Load(cargoTargetRelease);
            }

            dir = Path.GetDirectoryName(dir);
        }
#endif

        // Fallback to default resolution
        return IntPtr.Zero;
    }

    // -----------------------------------------------------------------------
    // Version
    // -----------------------------------------------------------------------

    /// <summary>Returns the FFI library version. Caller must free the result.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial IntPtr hyperlight_sandbox_get_version();

    // -----------------------------------------------------------------------
    // String management
    // -----------------------------------------------------------------------

    /// <summary>Frees a string allocated by the Rust FFI layer.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial void hyperlight_sandbox_free_string(IntPtr s);

    // -----------------------------------------------------------------------
    // Sandbox lifecycle
    // -----------------------------------------------------------------------

    /// <summary>Creates a new sandbox instance (not yet initialized).</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_create(
        FFISandboxOptions options);

    /// <summary>Frees a sandbox handle. Null is safe (no-op).</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial void hyperlight_sandbox_free(IntPtr handle);

    // -----------------------------------------------------------------------
    // Configuration (pre-run)
    // -----------------------------------------------------------------------

    /// <summary>Sets the read-only input directory.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_set_input_dir(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    /// <summary>Sets the writable output directory.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_set_output_dir(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string path);

    /// <summary>Enables/disables temporary output directory.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_set_temp_output(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.I1)] bool enabled);

    /// <summary>Adds a domain to the network allowlist.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_allow_domain(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string target,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? methodsJson);

    // -----------------------------------------------------------------------
    // Tool registration
    // -----------------------------------------------------------------------

    /// <summary>Registers a host-side tool callable from guest code.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_register_tool(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string? schemaJson,
        IntPtr callback);

    // -----------------------------------------------------------------------
    // Execution
    // -----------------------------------------------------------------------

    /// <summary>Executes guest code. Returns JSON ExecutionResult.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_run(
        SandboxSafeHandle handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string code);

    // -----------------------------------------------------------------------
    // Filesystem
    // -----------------------------------------------------------------------

    /// <summary>Returns output filenames as a JSON array.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_get_output_files(
        SandboxSafeHandle handle);

    /// <summary>Returns the host path of the output directory.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_output_path(
        SandboxSafeHandle handle);

    // -----------------------------------------------------------------------
    // Snapshot / Restore
    // -----------------------------------------------------------------------

    /// <summary>Takes a snapshot of the sandbox state.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_snapshot(
        SandboxSafeHandle handle);

    /// <summary>Restores the sandbox to a previous snapshot.</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial FFIResult hyperlight_sandbox_restore(
        SandboxSafeHandle handle,
        SnapshotSafeHandle snapshot);

    /// <summary>Frees a snapshot handle. Null is safe (no-op).</summary>
    [LibraryImport(LibName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial void hyperlight_sandbox_free_snapshot(IntPtr snapshot);
}

#pragma warning restore CA5392
