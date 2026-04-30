// Snapshot/restore example — fast sandbox reset without cold start.
//
// Demonstrates the snapshot/restore pattern used for agent tool calls:
// take a "warm" snapshot after initialization, then restore to it
// before each execution for a clean-but-fast environment.

using System.Diagnostics;
using HyperlightSandbox.Api;
using HyperlightSandbox.Examples.Common;

var guestPath = ExampleHelper.RequirePythonGuest();

Console.WriteLine("=== Hyperlight Sandbox .NET — Snapshot Example ===\n");

using var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .WithTempOutput()
    .Build();

// --- Cold start: first run initializes the sandbox ---
Console.WriteLine("═══ Step 1: Cold start (first run) ═══");
var sw = Stopwatch.StartNew();
var result = sandbox.Run("print('Cold start complete')");
sw.Stop();
Console.WriteLine($"  stdout: {result.Stdout.Trim()}");
Console.WriteLine($"  Cold start time: {sw.ElapsedMilliseconds}ms");

// --- Take a snapshot of the warm state ---
Console.WriteLine("\n═══ Step 2: Take snapshot ═══");
using var snapshot = sandbox.Snapshot();
Console.WriteLine("  Snapshot taken.");

// --- Modify state ---
Console.WriteLine("\n═══ Step 3: Modify state ═══");
sandbox.Run("""
    with open("/output/state.txt", "w") as f:
        f.write("modified state")
    print("State modified — wrote to /output/state.txt")
    """);
Console.WriteLine($"  Output files after modification: [{string.Join(", ", sandbox.GetOutputFiles())}]");

// --- Restore from snapshot —- state should be clean ---
Console.WriteLine("\n═══ Step 4: Restore from snapshot ═══");
sandbox.Restore(snapshot);
Console.WriteLine("  Snapshot restored.");

sw.Restart();
result = sandbox.Run("print('After restore — clean state')");
sw.Stop();
Console.WriteLine($"  stdout: {result.Stdout.Trim()}");
Console.WriteLine($"  Warm restore time: {sw.ElapsedMilliseconds}ms");

// --- Reuse the snapshot multiple times ---
Console.WriteLine("\n═══ Step 5: Reuse snapshot multiple times ═══");
for (int i = 1; i <= 3; i++)
{
    sandbox.Restore(snapshot);
    sw.Restart();
    result = sandbox.Run($"print(f'Iteration {i} from clean state')");
    sw.Stop();
    Console.WriteLine($"  Iteration {i}: {result.Stdout.Trim()} ({sw.ElapsedMilliseconds}ms)");
}

Console.WriteLine("\n✅ Snapshot example finished successfully!");
return 0;
