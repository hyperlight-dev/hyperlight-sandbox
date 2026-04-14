// Filesystem example — input/output directories and temp output.
//
// Mirrors: src/wasm_sandbox/examples/python_filesystem_demo.rs

using HyperlightSandbox.Api;
using HyperlightSandbox.Examples.Common;

var guestPath = ExampleHelper.RequirePythonGuest();

Console.WriteLine("=== Hyperlight Sandbox .NET — Filesystem Example ===\n");

// --- Test 1: Temp output ---
Console.WriteLine("═══ Test 1: Temp output directory ═══");
using (var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .WithTempOutput()
    .Build())
{
    sandbox.Run("""
        with open("/output/hello.txt", "w") as f:
            f.write("Hello from the sandbox!")
        print("Wrote hello.txt")
        """);

    var files = sandbox.GetOutputFiles();
    Console.WriteLine($"  Output files: [{string.Join(", ", files)}]");
    Console.WriteLine($"  Output path: {sandbox.OutputPath}");
}

// --- Test 2: Input directory ---
Console.WriteLine("\n═══ Test 2: Input directory ═══");

// Create a temp input directory with a test file.
var inputDir = Path.Combine(Path.GetTempPath(), $"hyperlight-input-{Guid.NewGuid():N}");
Directory.CreateDirectory(inputDir);
File.WriteAllText(Path.Combine(inputDir, "data.txt"), "Input data from host");

try
{
    using var sandbox = new SandboxBuilder()
        .WithModulePath(guestPath)
        .WithInputDir(inputDir)
        .WithTempOutput()
        .Build();

    var result = sandbox.Run("""
        with open("/input/data.txt", "r") as f:
            content = f.read()
        print(f"Read from input: {content}")

        with open("/output/processed.txt", "w") as f:
            f.write(f"Processed: {content.upper()}")
        print("Wrote processed.txt to output")
        """);

    Console.WriteLine($"  stdout: {result.Stdout.Trim()}");
    Console.WriteLine($"  Output files: [{string.Join(", ", sandbox.GetOutputFiles())}]");
}
finally
{
    Directory.Delete(inputDir, recursive: true);
}

Console.WriteLine("\n✅ Filesystem example finished successfully!");
return 0;
