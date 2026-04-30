using System.Runtime.InteropServices;

namespace HyperlightSandbox.PInvoke;

/// <summary>
/// Manages the lifecycle of a native sandbox handle.
/// Ensures the underlying Rust <c>SandboxState</c> is properly freed
/// when no longer needed.
/// </summary>
/// <remarks>
/// <para>
/// The handle is created by <see cref="SafeNativeMethods.hyperlight_sandbox_create"/>
/// and freed by <see cref="SafeNativeMethods.hyperlight_sandbox_free"/>.
/// </para>
/// <para>
/// <b>Ownership transfer</b>: When a consuming operation invalidates this
/// handle (e.g., a hypothetical reload), call <see cref="MakeHandleInvalid"/>
/// immediately after the FFI call to prevent the finalizer from double-freeing.
/// Follow with <see cref="GC.KeepAlive"/> on the owning object to prevent
/// premature finalization during the FFI call.
/// </para>
/// </remarks>
internal sealed class SandboxSafeHandle : SafeHandle
{
    public SandboxSafeHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public SandboxSafeHandle(IntPtr handle) : base(IntPtr.Zero, ownsHandle: true)
    {
        SetHandle(handle);
    }

    /// <summary>
    /// Marks this handle as invalid so the finalizer will not attempt to
    /// free it. Used after a consuming FFI call has taken ownership.
    /// </summary>
    public void MakeHandleInvalid()
    {
        SetHandle(IntPtr.Zero);
        SetHandleAsInvalid();
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    /// <summary>
    /// Releases the native sandbox resource.
    /// Uses <see cref="Interlocked.Exchange"/> to ensure exactly-once
    /// semantics even if the GC finalizer and an explicit <c>Dispose()</c>
    /// race.
    /// </summary>
    protected override bool ReleaseHandle()
    {
        IntPtr oldHandle = Interlocked.Exchange(ref handle, IntPtr.Zero);
        if (oldHandle != IntPtr.Zero)
        {
            SafeNativeMethods.hyperlight_sandbox_free(oldHandle);
        }

        return true;
    }
}

/// <summary>
/// Manages the lifecycle of a native snapshot handle.
/// Ensures the underlying Rust snapshot data is properly freed.
/// </summary>
/// <remarks>
/// Snapshots can be reused multiple times for restore operations.
/// The snapshot is only freed when this handle is disposed or finalized.
/// </remarks>
internal sealed class SnapshotSafeHandle : SafeHandle
{
    public SnapshotSafeHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public SnapshotSafeHandle(IntPtr handle) : base(IntPtr.Zero, ownsHandle: true)
    {
        SetHandle(handle);
    }

    /// <summary>
    /// Marks this handle as invalid so the finalizer will not attempt to
    /// free it.
    /// </summary>
    public void MakeHandleInvalid()
    {
        SetHandle(IntPtr.Zero);
        SetHandleAsInvalid();
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    /// <summary>
    /// Releases the native snapshot resource.
    /// Uses <see cref="Interlocked.Exchange"/> for race-free cleanup.
    /// </summary>
    protected override bool ReleaseHandle()
    {
        IntPtr oldHandle = Interlocked.Exchange(ref handle, IntPtr.Zero);
        if (oldHandle != IntPtr.Zero)
        {
            SafeNativeMethods.hyperlight_sandbox_free_snapshot(oldHandle);
        }

        return true;
    }
}
