namespace HyperlightSandbox.Api;

/// <summary>
/// The sandbox backend to use for code execution.
/// </summary>
public enum SandboxBackend
{
    /// <summary>
    /// WebAssembly component backend.
    /// Requires a <c>.wasm</c> or <c>.aot</c> guest module
    /// (e.g., Python compiled to Wasm).
    /// </summary>
    Wasm = 0,

    /// <summary>
    /// Hyperlight-JS built-in JavaScript backend.
    /// Uses an embedded QuickJS runtime — no module path needed.
    /// </summary>
    JavaScript = 1,
}
