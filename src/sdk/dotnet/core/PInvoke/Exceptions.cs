namespace HyperlightSandbox;

/// <summary>
/// Base exception for sandbox-related errors.
/// </summary>
public class SandboxException : Exception
{
    public SandboxException() { }
    public SandboxException(string message) : base(message) { }
    public SandboxException(string message, Exception innerException)
        : base(message, innerException) { }
}

/// <summary>
/// Thrown when sandbox execution exceeds a time limit.
/// </summary>
public sealed class SandboxTimeoutException : SandboxException
{
    public SandboxTimeoutException() { }
    public SandboxTimeoutException(string message) : base(message) { }
    public SandboxTimeoutException(string message, Exception innerException)
        : base(message, innerException) { }
}

/// <summary>
/// Thrown when the sandbox is in a poisoned state (e.g., mutex poisoned,
/// guest crash). The sandbox must be recreated.
/// </summary>
public sealed class SandboxPoisonedException : SandboxException
{
    public SandboxPoisonedException() { }
    public SandboxPoisonedException(string message) : base(message) { }
    public SandboxPoisonedException(string message, Exception innerException)
        : base(message, innerException) { }
}

/// <summary>
/// Thrown when a network operation is denied by the sandbox's permission policy.
/// </summary>
public sealed class SandboxPermissionException : SandboxException
{
    public SandboxPermissionException() { }
    public SandboxPermissionException(string message) : base(message) { }
    public SandboxPermissionException(string message, Exception innerException)
        : base(message, innerException) { }
}

/// <summary>
/// Thrown when guest code raises an error during execution.
/// </summary>
public sealed class SandboxGuestException : SandboxException
{
    public SandboxGuestException() { }
    public SandboxGuestException(string message) : base(message) { }
    public SandboxGuestException(string message, Exception innerException)
        : base(message, innerException) { }
}
