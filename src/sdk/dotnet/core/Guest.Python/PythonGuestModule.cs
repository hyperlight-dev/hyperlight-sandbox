using HyperlightSandbox.Api;

namespace HyperlightSandbox.Guest.Python;

/// <summary>
/// Provides access to the bundled Python guest module.
/// </summary>
public static class PythonGuestModule
{
    /// <summary>The bundled Python AOT module file name.</summary>
    public const string ModuleFileName = "python-sandbox.aot";

    /// <summary>
    /// Returns the path to the Python AOT guest module.
    /// Follows the NuGet <c>runtimes/{rid}/native/</c> probing convention
    /// (same approach as <c>SafeNativeMethods.DllImportResolver</c> in PInvoke).
    /// </summary>
    public static string GetModulePath() => FindGuestFile(ModuleFileName);

    /// <summary>
    /// Configures the builder to use the bundled Python guest module with the Wasm backend.
    /// </summary>
    /// <param name="builder">The sandbox builder to configure.</param>
    /// <returns>The same builder for chaining.</returns>
    public static SandboxBuilder WithPythonModule(this SandboxBuilder builder)
    {
        ArgumentNullException.ThrowIfNull(builder);
        return builder
            .WithBackend(SandboxBackend.Wasm)
            .WithModulePath(GetModulePath());
    }

    /// <summary>
    /// Configures the builder to use the bundled Python guest module with the Wasm backend.
    /// </summary>
    /// <param name="builder">The sandbox builder to configure.</param>
    /// <returns>The same builder for chaining.</returns>
    public static SandboxBuilder AddPythonModule(this SandboxBuilder builder) =>
        WithPythonModule(builder);

    private static string FindGuestFile(string fileName)
    {
        string assemblyDir = Path.GetDirectoryName(
            typeof(PythonGuestModule).Assembly.Location) ?? AppContext.BaseDirectory;

        // The .aot binary is built on Windows but works on both linux-x64 and win-x64.
        // Probe the OS-appropriate RID path first (matches SafeNativeMethods.DllImportResolver).
        string rid = OperatingSystem.IsWindows() ? "win-x64" : "linux-x64";
        string nativePath = Path.Join(assemblyDir, "runtimes", rid, "native", fileName);
        if (File.Exists(nativePath))
        {
            return nativePath;
        }

        // Flat output directory (some single-file / RID-specific publish layouts)
        string flatPath = Path.Join(assemblyDir, fileName);
        if (File.Exists(flatPath))
        {
            return flatPath;
        }

#if DEBUG
        // Walk up from assembly dir to find source guest build output (local dev without pack)
        string? dir = assemblyDir;
        while (dir != null)
        {
            string guestPath = Path.Join(dir, "src", "wasm_sandbox", "guests", "python", fileName);
            if (File.Exists(guestPath))
            {
                return guestPath;
            }

            dir = Path.GetDirectoryName(dir);
        }
#endif

        throw new FileNotFoundException(
            $"Python guest file '{fileName}' not found. " +
            $"Searched '{nativePath}'. " +
            "Ensure the package was built with 'just wasm guest-build'.",
            nativePath);
    }
}
