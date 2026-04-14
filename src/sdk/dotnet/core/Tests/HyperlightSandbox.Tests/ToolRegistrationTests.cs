using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for tool registration API — both raw JSON and typed variants.
/// Tests registration validation, schema generation, and error cases.
/// Full dispatch tests (calling tools from guest) require a real wasm module;
/// these tests validate the registration path and safety properties.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class ToolRegistrationTests
{
    private static Sandbox CreateTestSandbox() =>
        new SandboxBuilder()
            .WithModulePath("/tmp/test-tools.wasm")
            .Build();

    // -----------------------------------------------------------------------
    // Raw JSON tool registration
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_RawJson_Succeeds()
    {
        using var sandbox = CreateTestSandbox();
        sandbox.RegisterTool("echo", (string json) => json);
    }

    [Fact]
    public void RegisterTool_RawJson_NullName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool(null!, (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_RawJson_EmptyName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool("", (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_RawJson_NullHandler_ThrowsArgumentNullException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.Throws<ArgumentNullException>(() =>
            sandbox.RegisterTool("test", (Func<string, string>)null!));
    }

    // -----------------------------------------------------------------------
    // Typed tool registration
    // -----------------------------------------------------------------------

    private sealed class AddArgs
    {
        public double a { get; set; }
        public double b { get; set; }
    }

    private sealed class AddResult
    {
        public double sum { get; set; }
    }

    [Fact]
    public void RegisterTool_Typed_Succeeds()
    {
        using var sandbox = CreateTestSandbox();
        sandbox.RegisterTool<AddArgs, AddResult>("add",
            args => new AddResult { sum = args.a + args.b });
    }

    [Fact]
    public void RegisterTool_Typed_NullName_ThrowsArgumentException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.ThrowsAny<ArgumentException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>(null!,
                args => new AddResult { sum = 0 }));
    }

    [Fact]
    public void RegisterTool_Typed_NullHandler_ThrowsArgumentNullException()
    {
        using var sandbox = CreateTestSandbox();
        Assert.Throws<ArgumentNullException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>("add", null!));
    }

    // -----------------------------------------------------------------------
    // Multiple tool registration
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_MultipleDifferentNames_Succeeds()
    {
        using var sandbox = CreateTestSandbox();

        sandbox.RegisterTool("tool1", (string json) => "{}");
        sandbox.RegisterTool("tool2", (string json) => "{}");
        sandbox.RegisterTool("tool3", (string json) => "{}");
        sandbox.RegisterTool<AddArgs, AddResult>("add",
            args => new AddResult { sum = args.a + args.b });
    }

    // -----------------------------------------------------------------------
    // Registration after dispose
    // -----------------------------------------------------------------------

    [Fact]
    public void RegisterTool_AfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = CreateTestSandbox();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.RegisterTool("test", (string json) => "{}"));
    }

    [Fact]
    public void RegisterTool_Typed_AfterDispose_ThrowsObjectDisposedException()
    {
        var sandbox = CreateTestSandbox();
        sandbox.Dispose();

        Assert.Throws<ObjectDisposedException>(() =>
            sandbox.RegisterTool<AddArgs, AddResult>("add",
                args => new AddResult { sum = 0 }));
    }

    // -----------------------------------------------------------------------
    // Schema generation (ToolSchemaBuilder)
    // -----------------------------------------------------------------------

    [Fact]
    public void ToolSchemaBuilder_NumericTypes_MapToNumber()
    {
        var schema = ToolSchemaBuilder.BuildSchema<NumericArgs>();
        Assert.Contains("\"Number\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_StringType_MapsToString()
    {
        var schema = ToolSchemaBuilder.BuildSchema<StringArgs>();
        Assert.Contains("\"String\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_BoolType_MapsToBoolean()
    {
        var schema = ToolSchemaBuilder.BuildSchema<BoolArgs>();
        Assert.Contains("\"Boolean\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_ComplexType_MapsToObject()
    {
        var schema = ToolSchemaBuilder.BuildSchema<ComplexArgs>();
        Assert.Contains("\"Object\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_ArrayType_MapsToArray()
    {
        var schema = ToolSchemaBuilder.BuildSchema<ArrayArgs>();
        Assert.Contains("\"Array\"", schema, StringComparison.Ordinal);
    }

    [Fact]
    public void ToolSchemaBuilder_AllPropertiesRequired()
    {
        var schema = ToolSchemaBuilder.BuildSchema<AddArgs>();
        Assert.Contains("\"a\"", schema, StringComparison.Ordinal);
        Assert.Contains("\"b\"", schema, StringComparison.Ordinal);
        Assert.Contains("required", schema, StringComparison.Ordinal);
    }

    // Schema test helper types — instantiated by System.Text.Json reflection
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class NumericArgs { public int Value { get; set; } }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class StringArgs { public string Name { get; set; } = ""; }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class BoolArgs { public bool Flag { get; set; } }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class ComplexArgs { public AddArgs Nested { get; set; } = new(); }
    [System.Diagnostics.CodeAnalysis.SuppressMessage("Performance", "CA1812:Avoid uninstantiated internal classes", Justification = "Used as type parameter for schema generation")]
    private sealed class ArrayArgs { public int[] Items { get; set; } = []; }
}
