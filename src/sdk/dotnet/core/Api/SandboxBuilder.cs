namespace HyperlightSandbox.Api;

/// <summary>
/// A builder for creating <see cref="Sandbox"/> instances with custom
/// configuration.
/// </summary>
/// <remarks>
/// <para>
/// The builder can be reused to create multiple sandboxes with the same
/// configuration.
/// </para>
/// <example>
/// <code>
/// var sandbox = new SandboxBuilder()
///     .WithModulePath("/path/to/python-sandbox.aot")
///     .WithHeapSize("50Mi")
///     .WithTempOutput()
///     .Build();
/// </code>
/// </example>
/// </remarks>
public sealed class SandboxBuilder
{
    private string? _modulePath;
    private ulong _heapSize;
    private ulong _stackSize;
    private string? _inputDir;
    private string? _outputDir;
    private bool _tempOutput;
    private SandboxBackend _backend = SandboxBackend.Wasm;

    /// <summary>
    /// Sets the backend to use. Default is <see cref="SandboxBackend.Wasm"/>.
    /// </summary>
    /// <param name="backend">The backend type.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithBackend(SandboxBackend backend)
    {
        _backend = backend;
        return this;
    }

    /// <summary>
    /// Sets the path to the guest module (<c>.wasm</c> or <c>.aot</c> file).
    /// Required for <see cref="SandboxBackend.Wasm"/>, not needed for
    /// <see cref="SandboxBackend.JavaScript"/>.
    /// </summary>
    /// <param name="path">Absolute or relative path to the guest module.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithModulePath(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _modulePath = path;
        return this;
    }

    /// <summary>
    /// Sets the guest heap size.
    /// </summary>
    /// <param name="size">
    /// Size string (e.g. <c>"25Mi"</c>, <c>"2Gi"</c>) or raw bytes as string.
    /// </param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithHeapSize(string size)
    {
        _heapSize = SizeParser.Parse(size);
        return this;
    }

    /// <summary>
    /// Sets the guest heap size in bytes.
    /// </summary>
    /// <param name="bytes">Heap size in bytes.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithHeapSize(ulong bytes)
    {
        _heapSize = bytes;
        return this;
    }

    /// <summary>
    /// Sets the guest stack size.
    /// </summary>
    /// <param name="size">
    /// Size string (e.g. <c>"35Mi"</c>) or raw bytes as string.
    /// </param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithStackSize(string size)
    {
        _stackSize = SizeParser.Parse(size);
        return this;
    }

    /// <summary>
    /// Sets the guest stack size in bytes.
    /// </summary>
    /// <param name="bytes">Stack size in bytes.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithStackSize(ulong bytes)
    {
        _stackSize = bytes;
        return this;
    }

    /// <summary>
    /// Sets the host directory exposed as read-only <c>/input</c> inside
    /// the sandbox.
    /// </summary>
    /// <param name="path">Path to the input directory.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithInputDir(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _inputDir = path;
        return this;
    }

    /// <summary>
    /// Sets the host directory exposed as writable <c>/output</c> inside
    /// the sandbox.
    /// </summary>
    /// <param name="path">Path to the output directory.</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithOutputDir(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _outputDir = path;
        return this;
    }

    /// <summary>
    /// Enables a temporary writable <c>/output</c> directory.
    /// Ignored if <see cref="WithOutputDir"/> was called.
    /// </summary>
    /// <param name="enabled">Whether to enable temp output (default: true).</param>
    /// <returns>This builder for chaining.</returns>
    public SandboxBuilder WithTempOutput(bool enabled = true)
    {
        _tempOutput = enabled;
        return this;
    }

    /// <summary>
    /// Creates a new <see cref="Sandbox"/> with the configured settings.
    /// </summary>
    /// <returns>A new sandbox instance.</returns>
    /// <exception cref="InvalidOperationException">
    /// Thrown if <see cref="WithModulePath"/> was not called.
    /// </exception>
    /// <exception cref="SandboxException">
    /// Thrown if the native sandbox creation fails.
    /// </exception>
    public Sandbox Build()
    {
        if (_backend == SandboxBackend.Wasm && string.IsNullOrWhiteSpace(_modulePath))
        {
            throw new InvalidOperationException(
                "Module path is required for the Wasm backend. Call WithModulePath() before Build().");
        }

        if (_backend == SandboxBackend.JavaScript && !string.IsNullOrWhiteSpace(_modulePath))
        {
            throw new InvalidOperationException(
                "Module path must not be set for the JavaScript backend (it has a built-in runtime).");
        }

        return new Sandbox(
            _modulePath,
            _heapSize,
            _stackSize,
            _inputDir,
            _outputDir,
            _tempOutput,
            _backend);
    }
}
