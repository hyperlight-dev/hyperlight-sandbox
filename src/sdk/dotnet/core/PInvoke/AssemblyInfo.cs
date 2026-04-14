using System.Runtime.CompilerServices;

// Disable runtime marshaling to enable blittable type marshaling with LibraryImport.
// This allows the source generator to handle marshaling at compile time for better performance.
[assembly: DisableRuntimeMarshalling]

// Allow the Api project to access internal types for the high-level wrapper.
[assembly: InternalsVisibleTo("HyperlightSandbox.Api")]
// Allow the test project to access internal types for thorough testing.
[assembly: InternalsVisibleTo("HyperlightSandbox.Tests")]
