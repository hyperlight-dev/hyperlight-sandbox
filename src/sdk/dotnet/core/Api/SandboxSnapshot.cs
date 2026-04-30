using HyperlightSandbox.PInvoke;

namespace HyperlightSandbox.Api;

/// <summary>
/// Wraps a native snapshot handle, ensuring proper cleanup.
/// Snapshots can be reused for multiple <see cref="Sandbox.Restore"/> calls.
/// </summary>
/// <remarks>
/// <para>
/// Snapshots capture the memory state of the sandbox at a point in time.
/// Use <see cref="Sandbox.Snapshot"/> to create one, then
/// <see cref="Sandbox.Restore"/> to return the sandbox to that state.
/// </para>
/// <para>
/// This class implements <see cref="IDisposable"/>. Always dispose when
/// done to free native memory promptly.
/// </para>
/// </remarks>
public sealed class SandboxSnapshot : IDisposable
{
    internal readonly SnapshotSafeHandle Handle;

    internal SandboxSnapshot(SnapshotSafeHandle handle)
    {
        Handle = handle;
    }

    /// <summary>
    /// Returns <c>true</c> if the snapshot has been disposed.
    /// </summary>
    public bool IsDisposed => Handle.IsInvalid || Handle.IsClosed;

    /// <summary>Releases the native snapshot resource.</summary>
    public void Dispose()
    {
        Handle.Dispose();
    }
}
