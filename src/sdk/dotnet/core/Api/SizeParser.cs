namespace HyperlightSandbox.Api;

/// <summary>
/// Parses human-readable size strings (e.g. <c>"25Mi"</c>, <c>"2Gi"</c>)
/// into byte values. Matches the Python SDK's size format for consistency.
/// </summary>
/// <remarks>
/// Supported suffixes:
/// <list type="bullet">
/// <item><description><c>Ki</c> — kibibytes (×1024)</description></item>
/// <item><description><c>Mi</c> — mebibytes (×1024²)</description></item>
/// <item><description><c>Gi</c> — gibibytes (×1024³)</description></item>
/// <item><description>No suffix — raw bytes</description></item>
/// </list>
/// </remarks>
internal static class SizeParser
{
    /// <summary>
    /// Parses a size string to bytes.
    /// </summary>
    /// <param name="size">Size string like <c>"25Mi"</c> or <c>"1024"</c>.</param>
    /// <returns>Size in bytes.</returns>
    /// <exception cref="ArgumentException">
    /// Thrown if <paramref name="size"/> is null, empty, or not a valid size string.
    /// </exception>
    /// <exception cref="OverflowException">
    /// Thrown if the computed size overflows <see cref="ulong"/>.
    /// </exception>
    public static ulong Parse(string size)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(size);

        var trimmed = size.AsSpan().Trim();

        ulong multiplier;
        ReadOnlySpan<char> numberPart;

        if (trimmed.EndsWith("Gi", StringComparison.Ordinal))
        {
            multiplier = 1024UL * 1024 * 1024;
            numberPart = trimmed[..^2];
        }
        else if (trimmed.EndsWith("Mi", StringComparison.Ordinal))
        {
            multiplier = 1024UL * 1024;
            numberPart = trimmed[..^2];
        }
        else if (trimmed.EndsWith("Ki", StringComparison.Ordinal))
        {
            multiplier = 1024UL;
            numberPart = trimmed[..^2];
        }
        else
        {
            multiplier = 1;
            numberPart = trimmed;
        }

        if (!ulong.TryParse(numberPart, out var value))
        {
            throw new ArgumentException($"Invalid size string: '{size}'", nameof(size));
        }

        try
        {
            return checked(value * multiplier);
        }
        catch (OverflowException)
        {
            throw new OverflowException($"Size value overflows: '{size}'");
        }
    }
}
