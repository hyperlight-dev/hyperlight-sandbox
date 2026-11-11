using System.Runtime.InteropServices;

namespace HyperlightSandbox;

internal static class NativePlatform
{
    internal static string RuntimeIdentifier
    {
        get
        {
            string os = OperatingSystem.IsWindows() ? "win"
                : OperatingSystem.IsMacOS() ? "osx"
                : OperatingSystem.IsLinux() ? "linux"
                : throw new PlatformNotSupportedException("Hyperlight requires Windows, Linux, or macOS.");
            string arch = RuntimeInformation.ProcessArchitecture switch
            {
                Architecture.X64 => "x64",
                Architecture.Arm64 => "arm64",
                _ => throw new PlatformNotSupportedException(
                    $"Unsupported Hyperlight process architecture: {RuntimeInformation.ProcessArchitecture}."),
            };
            return $"{os}-{arch}";
        }
    }
}
