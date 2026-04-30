using System.Runtime.InteropServices;

namespace HyperlightSandbox.PInvoke;

/// <summary>
/// Configuration options for sandbox creation, matching the Rust
/// <c>FFISandboxOptions</c> struct layout.
/// </summary>
/// <remarks>
/// Zero values for <c>heap_size</c> and <c>stack_size</c> mean
/// "use platform defaults" (25 MiB heap / 35 MiB stack on Linux,
/// 400 MiB / 200 MiB on Windows).
/// </remarks>
[StructLayout(LayoutKind.Sequential)]
internal struct FFISandboxOptions : IEquatable<FFISandboxOptions>
{
    /// <summary>
    /// Pointer to the null-terminated UTF-8 module path string.
    /// Required for Wasm, must be IntPtr.Zero for JavaScript.
    /// </summary>
    public IntPtr module_path;

    /// <summary>Guest heap size in bytes. 0 = platform default.</summary>
    public ulong heap_size;

    /// <summary>Guest stack size in bytes. 0 = platform default.</summary>
    public ulong stack_size;

    /// <summary>Backend type: 0 = Wasm, 1 = JavaScript.</summary>
    public uint backend;

    public readonly bool Equals(FFISandboxOptions other)
    {
        return module_path == other.module_path
            && heap_size == other.heap_size
            && stack_size == other.stack_size
            && backend == other.backend;
    }

    public override readonly bool Equals(object? obj)
    {
        return obj is FFISandboxOptions other && Equals(other);
    }

    public override readonly int GetHashCode()
    {
        return HashCode.Combine(module_path, heap_size, stack_size, backend);
    }

    public static bool operator ==(FFISandboxOptions left, FFISandboxOptions right)
    {
        return left.Equals(right);
    }

    public static bool operator !=(FFISandboxOptions left, FFISandboxOptions right)
    {
        return !left.Equals(right);
    }
}
