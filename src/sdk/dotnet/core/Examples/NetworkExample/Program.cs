// Network example — HTTP allowlist and guest HTTP requests.
//
// Mirrors: src/wasm_sandbox/examples/python_network_demo.rs

using HyperlightSandbox.Api;
using HyperlightSandbox.Examples.Common;

var guestPath = ExampleHelper.RequirePythonGuest();

Console.WriteLine("=== Hyperlight Sandbox .NET — Network Example ===\n");

// --- Test 1: Allow a domain with all methods ---
Console.WriteLine("═══ Test 1: HTTP GET with allowlisted domain ═══");
using (var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .Build())
{
    sandbox.AllowDomain("https://httpbin.org");

    var result = sandbox.Run("""
        response = http_get("https://httpbin.org/get")
        print(f"Status: {response['status']}")
        print(f"Has headers: {'headers' in response}")
        """);

    Console.WriteLine($"  stdout: {result.Stdout.Trim()}");
    Console.WriteLine($"  success: {result.Success}");
}

// --- Test 2: Method-filtered access ---
Console.WriteLine("\n═══ Test 2: Method-filtered access (GET only) ═══");
using (var sandbox = new SandboxBuilder()
    .WithModulePath(guestPath)
    .Build())
{
    // Only allow GET requests to httpbin.org.
    sandbox.AllowDomain("https://httpbin.org", ["GET"]);

    var result = sandbox.Run("""
        # GET should succeed
        response = http_get("https://httpbin.org/get")
        print(f"GET status: {response['status']}")

        # POST should fail (method not allowed)
        try:
            response = http_post("https://httpbin.org/post", body="test")
            print(f"POST status: {response['status']}")
        except Exception as e:
            print(f"POST blocked: {e}")
        """);

    Console.WriteLine($"  stdout:\n{result.Stdout}");
}

Console.WriteLine("✅ Network example finished successfully!");
return 0;
