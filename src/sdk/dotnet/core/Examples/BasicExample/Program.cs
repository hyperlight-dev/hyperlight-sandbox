// Basic example — execute Python code in a secure sandbox.
//
// Mirrors: src/wasm_sandbox/examples/python_basics.rs
//
// Prerequisites:
//   just wasm guest-build    # builds python-sandbox.aot
//   just dotnet build        # builds the .NET SDK + FFI

using HyperlightSandbox.Api;
using HyperlightSandbox.Examples.Common;

var guestPath = ExampleHelper.RequirePythonGuest();

Console.WriteLine("=== Hyperlight Sandbox .NET — Basic Example ===\n");
Console.WriteLine($"Guest module: {guestPath}\n");

using var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .Build();

// --- Test 1: Basic code execution ---
Console.WriteLine("═══ Test 1: Basic code execution ═══");
var result = sandbox.Run("""
    import math
    primes = [n for n in range(2, 50) if all(n % i != 0 for i in range(2, int(math.sqrt(n)) + 1))]
    print(f"Primes under 50: {primes}")
    print(f"Count: {len(primes)}")
    """);

Console.WriteLine($"stdout: {result.Stdout}");
Console.WriteLine($"stderr: {result.Stderr}");
Console.WriteLine($"exit_code: {result.ExitCode}");
Console.WriteLine($"success: {result.Success}\n");

// --- Test 2: Multiple runs ---
Console.WriteLine("═══ Test 2: Multiple sequential runs ═══");
for (int i = 1; i <= 3; i++)
{
    var r = sandbox.Run($"print('Run {i}: Hello from the sandbox!')");
    Console.WriteLine($"  Run {i}: {r.Stdout.Trim()}");
}

Console.WriteLine("\n✅ Basic example finished successfully!");
return 0;
