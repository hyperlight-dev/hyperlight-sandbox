using HyperlightSandbox.Api;

namespace HyperlightSandbox.Guest.JavaScript;

/// <summary>
/// Provides access to the bundled JavaScript guest module.
/// </summary>
public static class JavaScriptGuestModule
{
    /// <summary>The bundled JavaScript AOT module file name.</summary>
    public const string ModuleFileName = "js-sandbox.aot";

    /// <summary>
    /// Returns the path to the JavaScript AOT guest module.
    /// Follows the NuGet <c>runtimes/{rid}/native/</c> probing convention
    /// (same approach as <c>SafeNativeMethods.DllImportResolver</c> in PInvoke).
    /// </summary>
    public static string GetModulePath() => FindGuestFile(ModuleFileName);

    /// <summary>
    /// Configures the builder to use the bundled JavaScript guest module with the Wasm backend.
    /// </summary>
    /// <param name="builder">The sandbox builder to configure.</param>
    /// <returns>The same builder for chaining.</returns>
    public static SandboxBuilder WithJavaScriptModule(this SandboxBuilder builder)
    {
        ArgumentNullException.ThrowIfNull(builder);
        return builder
            .WithBackend(SandboxBackend.Wasm)
            .WithModulePath(GetModulePath());
    }

    private static string FindGuestFile(string fileName)
    {
        string assemblyDir = Path.GetDirectoryName(
            typeof(JavaScriptGuestModule).Assembly.Location) ?? AppContext.BaseDirectory;

        string rid = NativePlatform.RuntimeIdentifier;
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
            string guestPath = Path.Join(dir, "src", "wasm_sandbox", "guests", "javascript", fileName);
            if (File.Exists(guestPath))
            {
                return guestPath;
            }

            dir = Path.GetDirectoryName(dir);
        }
#endif

        throw new FileNotFoundException(
            $"JavaScript guest file '{fileName}' not found. " +
            $"Searched '{nativePath}'. " +
            "Ensure the package was built with 'just wasm js-guest-build'.",
            nativePath);
    }
}
