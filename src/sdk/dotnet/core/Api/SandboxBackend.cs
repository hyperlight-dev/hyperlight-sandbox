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

    /// <summary>
    /// hyperlight-unikraft backend: full CPython in a Unikraft micro-VM.
    /// Requires the path to a rootfs image (initrd CPIO), e.g. the published
    /// <c>agent</c> image with numpy, pandas, scipy and scikit-learn.
    /// Its guest memory is set with <see cref="SandboxBuilder.WithScratchSize(string)"/>;
    /// heap and stack sizes are refused. Tool arguments and
    /// results and HTTP bodies of any size are exchanged in pieces, up to
    /// 64 MiB held unread on the host at a time. See
    /// <see cref="SandboxBuilder.WithNativeNetwork"/> for guest sockets.
    /// Exit codes are those a CPython process exits with, 0 to 255
    /// (<c>sys.exit(-1)</c> is 255), so -1 means the guest itself failed.
    /// Before each run the output directory is cleared of every file and
    /// link and of the directories the guest made; those it had when the
    /// sandbox was built are kept. A run that leaves a link, a special file,
    /// or files past the filesystem limits there fails, and is cleared too.
    /// </summary>
    Unikraft = 2,
}
