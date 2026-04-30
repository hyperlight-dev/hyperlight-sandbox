// .NET Agent example — hyperlight sandbox as an IChatClient tool.
//
// Mirrors: examples/agent-framework/copilot_agent.py
//
// Uses GitHub Models (OpenAI-compatible) as the LLM provider with
// IChatClient + FunctionInvocation for automatic tool calling.
//
// Usage:
//   GITHUB_TOKEN=ghp_... dotnet run --project examples/agent-framework/DotnetAgent.csproj
//
// Prerequisites:
//   just wasm guest-build   # build the Python guest module
//   just dotnet build       # build the .NET SDK
//   GITHUB_TOKEN env var    # GitHub PAT with Models access

using System.Text.Json.Serialization;
using HyperlightSandbox.Api;
using HyperlightSandbox.Extensions.AI;
using Microsoft.Extensions.AI;
using OpenAI;

// --- Check for GitHub token ---
var githubToken = Environment.GetEnvironmentVariable("GITHUB_TOKEN")
    ?? Environment.GetEnvironmentVariable("COPILOT_GITHUB_TOKEN");
if (string.IsNullOrEmpty(githubToken))
{
    Console.WriteLine("❌ Set GITHUB_TOKEN or COPILOT_GITHUB_TOKEN environment variable.");
    return 1;
}

// --- Find the guest module ---
var guestPath = FindGuest();
if (guestPath == null)
{
    Console.WriteLine("❌ Guest module not found. Run 'just wasm guest-build' first.");
    return 1;
}

Console.WriteLine("=== Hyperlight Sandbox .NET — Agent Example (IChatClient + FunctionInvocation) ===\n");

// --- Set up the sandbox code execution tool ---
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
        "divide" when args.B != 0 => args.A / args.B,
        _ => throw new ArgumentException($"Unknown operation: {args.Operation}"),
    });

// Async tool — simulates fetching from an external service.
codeTool.RegisterToolAsync<FetchDataArgs, string>("fetch_data",
    async args =>
    {
        // In real system this would be an actual HTTP/DB call.
        await Task.Delay(1).ConfigureAwait(false);
        return args.Source switch
        {
            "weather" => """{"temperature": 22, "condition": "sunny"}""",
            "stock" => """{"symbol": "MSFT", "price": 425.50}""",
            _ => """{"error": "unknown source"}""",
        };
    });

// --- Create IChatClient with function invocation ---
// GitHub Models provides an OpenAI-compatible endpoint.
var openAiClient = new OpenAIClient(
    new System.ClientModel.ApiKeyCredential(githubToken),
    new OpenAIClientOptions { Endpoint = new Uri("https://models.inference.ai.azure.com") });

IChatClient chatClient = new ChatClientBuilder(
        openAiClient.GetChatClient("gpt-4o").AsIChatClient())
    .UseFunctionInvocation()  // Automatically calls our tools when the model requests them
    .Build();

var chatOptions = new ChatOptions
{
    Tools = [codeTool.AsAIFunction()],
};

// --- System prompt (same approach as Python version) ---
var messages = new List<ChatMessage>
{
    new(ChatRole.System, """
        You have one tool: execute_code. It runs Python in an isolated sandbox.
        The sandbox has these built-in functions (no import needed):
        - call_tool("compute", a=<num>, b=<num>, operation="add"|"multiply"|"subtract"|"divide")
        - call_tool("fetch_data", source="weather"|"stock")
        Always use execute_code to perform computations. Never hardcode results.
        """),
};

// --- Run prompts through the agent ---
var prompts = new[]
{
    "Use execute_code to compute 42 * 17 using call_tool('compute', a=42, b=17, operation='multiply') and print the result.",
    "Use execute_code to fetch weather data using call_tool('fetch_data', source='weather') and print it nicely.",
};

foreach (var prompt in prompts)
{
    Console.WriteLine($"📤 User: {prompt}\n");
    messages.Add(new(ChatRole.User, prompt));

    var response = await chatClient.GetResponseAsync(messages, chatOptions).ConfigureAwait(false);

    Console.WriteLine($"🤖 Agent: {response.Messages.Last().Text}\n");
    messages.AddMessages(response);
    Console.WriteLine(new string('─', 60) + "\n");
}

Console.WriteLine("✅ Agent example finished!");
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
