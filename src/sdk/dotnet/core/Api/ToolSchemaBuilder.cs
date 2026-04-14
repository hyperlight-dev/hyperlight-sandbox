using System.Reflection;
using System.Text.Json;

namespace HyperlightSandbox.Api;

/// <summary>
/// Generates tool argument schemas from .NET types via reflection.
/// Used by <see cref="Sandbox.RegisterTool{TArgs,TResult}"/> to
/// auto-create the JSON schema passed to the Rust FFI layer.
/// </summary>
internal static class ToolSchemaBuilder
{
    /// <summary>
    /// Builds a JSON schema string from the public properties of
    /// <typeparamref name="TArgs"/>. All properties are treated as required.
    /// </summary>
    /// <typeparam name="TArgs">
    /// The type whose public properties define the tool's arguments.
    /// </typeparam>
    /// <returns>
    /// A JSON string like:
    /// <c>{"args": {"a": "Number", "b": "String"}, "required": ["a", "b"]}</c>
    /// </returns>
    public static string BuildSchema<TArgs>()
    {
        var type = typeof(TArgs);
        var args = new Dictionary<string, string>();
        var required = new List<string>();

        foreach (var prop in type.GetProperties(BindingFlags.Public | BindingFlags.Instance))
        {
            var argType = MapType(prop.PropertyType);
            // Use the JSON property name if available, otherwise the C# name
            var jsonName = GetJsonPropertyName(prop);
            args[jsonName] = argType;
            required.Add(jsonName);
        }

        var schema = new Dictionary<string, object>
        {
            ["args"] = args,
            ["required"] = required,
        };

        return JsonSerializer.Serialize(schema);
    }

    /// <summary>
    /// Maps a .NET type to the FFI schema type name.
    /// </summary>
    private static string MapType(Type type)
    {
        // Unwrap Nullable<T>
        var underlying = Nullable.GetUnderlyingType(type) ?? type;

        if (underlying == typeof(int)
            || underlying == typeof(long)
            || underlying == typeof(float)
            || underlying == typeof(double)
            || underlying == typeof(decimal)
            || underlying == typeof(short)
            || underlying == typeof(byte)
            || underlying == typeof(uint)
            || underlying == typeof(ulong)
            || underlying == typeof(ushort))
        {
            return "Number";
        }

        if (underlying == typeof(string))
        {
            return "String";
        }

        if (underlying == typeof(bool))
        {
            return "Boolean";
        }

        if (underlying.IsArray
            || (underlying.IsGenericType
                && underlying.GetGenericTypeDefinition() == typeof(List<>)))
        {
            return "Array";
        }

        // Default: treat complex types as Object
        return "Object";
    }

    /// <summary>
    /// Gets the JSON property name for a property, respecting
    /// <see cref="System.Text.Json.Serialization.JsonPropertyNameAttribute"/>.
    /// </summary>
    private static string GetJsonPropertyName(PropertyInfo prop)
    {
        var attr = prop.GetCustomAttribute<System.Text.Json.Serialization.JsonPropertyNameAttribute>();
        return attr?.Name ?? prop.Name;
    }
}
