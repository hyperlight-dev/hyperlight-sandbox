// Tool registration example — register host functions callable from guest code.
//
// Mirrors: src/wasm_sandbox/examples/python_basics.rs (tool dispatch section)

using System.Text.Json.Serialization;
using HyperlightSandbox.Api;
using HyperlightSandbox.Examples.Common;

var guestPath = ExampleHelper.RequirePythonGuest();

Console.WriteLine("=== Hyperlight Sandbox .NET — Tool Registration Example ===\n");

using var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .Build();

// --- Register typed tools ---
sandbox.RegisterTool<MathArgs, double>("add", args => args.A + args.B);
sandbox.RegisterTool<MathArgs, double>("multiply", args => args.A * args.B);
sandbox.RegisterTool<GreetArgs, string>("greet", args => $"Hello, {args.Name}!");

// --- Register a raw JSON tool ---
sandbox.RegisterTool("lookup", (string json) =>
{
    // Simple key-value lookup.
    if (json.Contains("api_key"))
        return """{"result": "sk-demo-12345"}""";
    if (json.Contains("model"))
        return """{"result": "gpt-4"}""";
    return """{"result": "not found"}""";
});

// --- Test 1: Typed tool dispatch ---
Console.WriteLine("═══ Test 1: Typed tool dispatch ═══");
var result = sandbox.Run("""
    sum_result = call_tool("add", a=10, b=32)
    print(f"10 + 32 = {sum_result}")

    product = call_tool("multiply", a=7, b=6)
    print(f"7 × 6 = {product}")

    greeting = call_tool("greet", name="Hyperlight")
    print(f"Greeting: {greeting}")
    """);

Console.WriteLine($"stdout:\n{result.Stdout}");

// --- Test 2: Raw JSON tool dispatch ---
Console.WriteLine("═══ Test 2: Raw JSON tool dispatch ═══");
result = sandbox.Run("""
    key = call_tool("lookup", key="api_key")
    print(f"API key: {key}")

    model = call_tool("lookup", key="model")
    print(f"Model: {model}")
    """);

Console.WriteLine($"stdout:\n{result.Stdout}");

Console.WriteLine("✅ Tool registration example finished successfully!");
return 0;

// --- DTOs ---
internal sealed class MathArgs
{
    [JsonPropertyName("a")]
    public double A { get; set; }

    [JsonPropertyName("b")]
    public double B { get; set; }
}

internal sealed class GreetArgs
{
    [JsonPropertyName("name")]
    public string Name { get; set; } = "world";
}
