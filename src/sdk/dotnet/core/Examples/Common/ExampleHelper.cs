namespace HyperlightSandbox.Examples.Common;

/// <summary>
/// Shared utilities for .NET SDK examples.
/// </summary>
public static class ExampleHelper
{
    /// <summary>
    /// Finds the Python guest module by walking up from the executing assembly's
    /// directory until we find the repo root (identified by <c>Cargo.toml</c>).
    /// </summary>
    /// <returns>Absolute path to <c>python-sandbox.aot</c>, or null if not found.</returns>
    public static string? FindPythonGuest()
    {
        var dir = AppContext.BaseDirectory;
        while (dir != null)
        {
            if (File.Exists(Path.Combine(dir, "Cargo.toml"))
                && Directory.Exists(Path.Combine(dir, "src", "wasm_sandbox")))
            {
                var guestPath = Path.Combine(dir,
                    "src", "wasm_sandbox", "guests", "python", "python-sandbox.aot");
                return File.Exists(guestPath) ? Path.GetFullPath(guestPath) : null;
            }

            dir = Path.GetDirectoryName(dir);
        }

        return null;
    }

    /// <summary>
    /// Gets the guest path or prints an error and exits.
    /// </summary>
    public static string RequirePythonGuest()
    {
        var path = FindPythonGuest();
        if (path == null)
        {
            Console.WriteLine("❌ Guest module not found. Run 'just wasm guest-build' first.");
            Environment.Exit(1);
        }

        return path;
    }
}
