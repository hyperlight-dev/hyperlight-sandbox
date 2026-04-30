namespace HyperlightSandbox.Api;

/// <summary>
/// The result of executing code inside the sandbox.
/// </summary>
/// <param name="Stdout">Standard output captured from the guest.</param>
/// <param name="Stderr">Standard error captured from the guest.</param>
/// <param name="ExitCode">Exit code from the guest process (0 = success).</param>
public sealed record ExecutionResult(string Stdout, string Stderr, int ExitCode)
{
    /// <summary>
    /// Returns <c>true</c> if the guest exited with code 0.
    /// </summary>
    public bool Success => ExitCode == 0;
}
