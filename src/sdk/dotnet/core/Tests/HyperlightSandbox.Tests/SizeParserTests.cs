using HyperlightSandbox.Api;
using Xunit;

namespace HyperlightSandbox.Tests;

/// <summary>
/// Tests for <see cref="SizeParser"/>.
/// </summary>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Design", "CA1515:Consider making public types internal", Justification = "Test classes must be public for xUnit")]
public class SizeParserTests
{
    [Fact]
    public void Parse_PlainBytes()
    {
        Assert.Equal(1024UL, SizeParser.Parse("1024"));
    }

    [Fact]
    public void Parse_Kibibytes()
    {
        Assert.Equal(10UL * 1024, SizeParser.Parse("10Ki"));
    }

    [Fact]
    public void Parse_Mebibytes()
    {
        Assert.Equal(25UL * 1024 * 1024, SizeParser.Parse("25Mi"));
    }

    [Fact]
    public void Parse_Gibibytes()
    {
        Assert.Equal(2UL * 1024 * 1024 * 1024, SizeParser.Parse("2Gi"));
    }

    [Theory]
    [InlineData("  400Mi  ")]
    [InlineData("\t25Mi\t")]
    public void Parse_WithWhitespace_TrimsCorrectly(string input)
    {
        Assert.True(SizeParser.Parse(input) > 0);
    }

    [Fact]
    public void Parse_LargeValue_WorksWithinBounds()
    {
        // 16 Gi — well within u64 range
        Assert.Equal(16UL * 1024 * 1024 * 1024, SizeParser.Parse("16Gi"));
    }

    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData(null)]
    public void Parse_NullOrEmpty_ThrowsArgumentException(string? input)
    {
        Assert.ThrowsAny<ArgumentException>(() => SizeParser.Parse(input!));
    }

    [Theory]
    [InlineData("abcMi")]
    [InlineData("Mi")]
    [InlineData("notanumber")]
    [InlineData("12.5Mi")]
    public void Parse_InvalidNumber_ThrowsArgumentException(string input)
    {
        Assert.ThrowsAny<ArgumentException>(() => SizeParser.Parse(input));
    }

    [Fact]
    public void Parse_Overflow_ThrowsOverflowException()
    {
        Assert.Throws<OverflowException>(() => SizeParser.Parse("999999999999999999Gi"));
    }

    [Fact]
    public void Parse_Zero_ReturnsZero()
    {
        Assert.Equal(0UL, SizeParser.Parse("0"));
    }

    [Fact]
    public void Parse_ZeroWithSuffix_ReturnsZero()
    {
        Assert.Equal(0UL, SizeParser.Parse("0Mi"));
    }
}
