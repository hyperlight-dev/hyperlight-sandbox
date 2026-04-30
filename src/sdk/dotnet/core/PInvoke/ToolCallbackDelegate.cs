using System.Runtime.InteropServices;

namespace HyperlightSandbox.PInvoke;

/// <summary>
/// Callback delegate invoked by the Rust FFI layer when guest code calls
/// <c>call_tool()</c>.
///
/// The callback receives a JSON-encoded arguments string and must return
/// a JSON-encoded result string.
/// </summary>
/// <param name="argsJson">
/// Pointer to a null-terminated UTF-8 JSON string containing the tool
/// arguments. Owned by the Rust caller — do NOT free this pointer.
/// </param>
/// <returns>
/// Pointer to a null-terminated UTF-8 JSON string containing the result.
/// The pointer must be allocated with <see cref="Marshal.StringToCoTaskMemUTF8"/>
/// and will be read then freed by the Rust side.
///
/// On error, return a JSON object with an <c>"error"</c> field:
/// <c>{"error": "description"}</c>.
///
/// Returning <see cref="IntPtr.Zero"/> is treated as an error by the
/// Rust layer.
/// </returns>
/// <remarks>
/// <para>
/// <b>Lifetime</b>: The delegate instance passed to
/// <see cref="SafeNativeMethods.hyperlight_sandbox_register_tool"/> MUST
/// be pinned via <see cref="GCHandle.Alloc(object)"/> for the entire
/// lifetime of the sandbox. If the GC collects the delegate while Rust
/// holds the function pointer, calling it will cause a SIGSEGV.
/// </para>
/// <para>
/// The API layer (<c>Sandbox.RegisterTool</c>) handles pinning
/// automatically — end users should not interact with this delegate
/// directly.
/// </para>
/// </remarks>
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal unsafe delegate IntPtr ToolCallbackDelegate(IntPtr argsJson);
