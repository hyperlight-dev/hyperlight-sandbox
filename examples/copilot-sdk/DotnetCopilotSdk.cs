// .NET Copilot SDK example — hyperlight sandbox as a Copilot tool.
//
// Mirrors: examples/copilot-sdk/copilot_sdk_tools.py
//
// Usage:
//   dotnet run --project examples/copilot-sdk/DotnetCopilotSdk.csproj
//
// Prerequisites:
//   just wasm guest-build   # build the Python guest module
//   just dotnet build       # build the .NET SDK
//   GitHub Copilot CLI installed and authenticated

using System.ComponentModel;
using System.Text.Json.Serialization;
using GitHub.Copilot.SDK;
using HyperlightSandbox.Api;
using HyperlightSandbox.Extensions.AI;
using Microsoft.Extensions.AI;

// --- Find the guest module ---
var guestPath = FindGuest();
if (guestPath == null)
{
    Console.WriteLine("❌ Guest module not found. Run 'just wasm guest-build' first.");
    return 1;
}

Console.WriteLine("=== Hyperlight Sandbox .NET — Copilot SDK Example ===\n");
Console.WriteLine($"Guest: {guestPath}\n");

// --- Set up the sandbox ---
using var codeTool = new CodeExecutionTool(
    new SandboxBuilder()
        .WithModulePath(guestPath)
        .WithTempOutput());

codeTool.RegisterTool<ComputeArgs, double>("compute",
    args => args.Operation switch
    {
        "add" => args.A + args.B,
        "multiply" => args.A * args.B,
        "subtract" => args.A - args.B,
        _ => throw new ArgumentException($"Unknown op: {args.Operation}"),
    });

codeTool.RegisterTool<FetchDataArgs, string>("fetch_data",
    args => args.Source switch
    {
        "weather" => """{"temperature": 22, "condition": "sunny"}""",
        "stock" => """{"symbol": "MSFT", "price": 425.50}""",
        _ => """{"error": "unknown source"}""",
    });

codeTool.AllowDomain("https://httpbin.org", ["GET"]);

// --- Connect to Copilot ---
Console.WriteLine("Connecting to GitHub Copilot CLI...\n");

await using var client = new CopilotClient();
await client.StartAsync().ConfigureAwait(false);

await using var session = await client.CreateSessionAsync(new SessionConfig
{
    Model = "claude-sonnet-4.5",
    OnPermissionRequest = PermissionHandler.ApproveAll,
    Tools =
    [
        codeTool.AsAIFunction(),
        AIFunctionFactory.Create(
            ([Description("Math expression")] string expr) => $"Computed: {expr}",
            "direct_compute",
            "Evaluate a math expression directly"),
    ],
    SystemMessage = new SystemMessageConfig
    {
        Mode = SystemMessageMode.Append,
        Content = """
            You have access to an execute_code tool that runs Python code in a
            secure sandbox. Available guest functions:
            - call_tool("compute", a=<num>, b=<num>, operation=<str>)
            - call_tool("fetch_data", source=<str>)
            - http_get(url)  (httpbin.org allowed)
            Always use execute_code for computation.
            """,
    },
}).ConfigureAwait(false);

// --- Send a prompt ---
var done = new TaskCompletionSource();

session.On(evt =>
{
    switch (evt)
    {
        case AssistantMessageEvent msg:
            Console.WriteLine($"\n🤖 {msg.Data.Content}\n");
            break;
        case ToolExecutionStartEvent toolStart:
            Console.WriteLine($"  🔧 Tool: {toolStart.Data.ToolName}");
            break;
        case SessionIdleEvent:
            done.TrySetResult();
            break;
        case SessionErrorEvent err:
            Console.WriteLine($"  ❌ Error: {err.Data.Message}");
            done.TrySetResult();
            break;
    }
});

Console.WriteLine("📤 Sending prompt...\n");
await session.SendAsync(new MessageOptions
{
    Prompt = "Use execute_code to compute 42 * 17 using call_tool('compute', a=42, b=17, operation='multiply') and print the result.",
}).ConfigureAwait(false);

await done.Task.ConfigureAwait(false);

Console.WriteLine("✅ Copilot SDK example finished!");
return 0;

// --- Helpers ---
static string? FindGuest()
{
    var dir = AppContext.BaseDirectory;
    while (dir != null)
    {
        if (File.Exists(Path.Combine(dir, "Cargo.toml"))
            && Directory.Exists(Path.Combine(dir, "src", "wasm_sandbox")))
        {
            var p = Path.Combine(dir, "src", "wasm_sandbox", "guests", "python", "python-sandbox.aot");
            return File.Exists(p) ? Path.GetFullPath(p) : null;
        }
        dir = Path.GetDirectoryName(dir);
    }
    return null;
}

internal sealed class ComputeArgs
{
    [JsonPropertyName("a")] public double A { get; set; }
    [JsonPropertyName("b")] public double B { get; set; }
    [JsonPropertyName("operation")] public string Operation { get; set; } = "add";
}

internal sealed class FetchDataArgs
{
    [JsonPropertyName("source")] public string Source { get; set; } = "";
}
