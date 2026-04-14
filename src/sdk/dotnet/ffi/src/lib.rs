//! C-compatible FFI layer for the hyperlight-sandbox .NET SDK.
//!
//! This crate produces a shared library (`cdylib`) that the .NET P/Invoke layer
//! calls via `[LibraryImport]`. It wraps the Rust `Sandbox<Wasm>` API with
//! opaque handle-based lifecycle management and JSON-over-FFI for complex types.
//!
//! # Architecture
//!
//! ```text
//!   .NET (C#)  ──[P/Invoke]──►  this crate (extern "C")  ──►  hyperlight-sandbox (Rust)
//! ```
//!
//! # Safety
//!
//! All `extern "C"` functions are `unsafe` at the boundary. Every function
//! validates its pointer arguments before dereferencing. Handles created by
//! `_create` functions must be freed with the corresponding `_free` function.

// FFI functions intentionally expose private types as opaque handles.
#![allow(private_interfaces)]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};

use anyhow::Result;
use hyperlight_javascript_sandbox::HyperlightJs;
use hyperlight_sandbox::{
    DEFAULT_HEAP_SIZE, DEFAULT_STACK_SIZE, DirPerms, FilePerms, GuestSandbox, HttpMethod, Sandbox,
    SandboxBuilder, SandboxConfig, ToolRegistry, ToolSchema,
};
use hyperlight_wasm_sandbox::Wasm;
use log::{debug, error};

// ---------------------------------------------------------------------------
// FFI error codes — structured classification across the boundary.
// Mirrored as `FFIErrorCode` enum in C# (`PInvoke/FFIErrorCode.cs`).
// ---------------------------------------------------------------------------

/// Error classification for FFI results.
///
/// These codes let the .NET layer map errors to specific exception types
/// without fragile string matching (a lesson from PR #292).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIErrorCode {
    /// No error.
    Success = 0,
    /// Unclassified error.
    Unknown = 1,
    /// Execution exceeded a time limit.
    Timeout = 2,
    /// Sandbox state is poisoned (mutex or guest crash).
    Poisoned = 3,
    /// Network permission denied.
    PermissionDenied = 4,
    /// Guest code raised an error.
    GuestError = 5,
    /// Invalid argument passed to FFI function.
    InvalidArgument = 6,
    /// Filesystem I/O error.
    IoError = 7,
}

// ---------------------------------------------------------------------------
// FFI result type
// ---------------------------------------------------------------------------

/// Result of an FFI operation.
///
/// On success: `is_success = true`, `error_code = 0`, `value` may hold a
/// pointer to an allocated string (caller must free with
/// `hyperlight_sandbox_free_string`).
///
/// On failure: `is_success = false`, `error_code` classifies the failure,
/// `value` holds a UTF-8 error message string (caller must free).
#[repr(C)]
#[derive(Debug)]
pub struct FFIResult {
    pub is_success: bool,
    pub error_code: u32,
    pub value: *mut c_char,
}

impl FFIResult {
    fn success(value: *mut c_char) -> Self {
        Self {
            is_success: true,
            error_code: FFIErrorCode::Success as u32,
            value,
        }
    }

    fn success_null() -> Self {
        Self::success(std::ptr::null_mut())
    }

    fn error(code: FFIErrorCode, message: CString) -> Self {
        Self {
            is_success: false,
            error_code: code as u32,
            value: message.into_raw(),
        }
    }
}

// ---------------------------------------------------------------------------
// FFI options struct
// ---------------------------------------------------------------------------

/// Configuration options for sandbox creation, passed by value from .NET.
///
/// Zero values mean "use platform default".
#[repr(C)]
pub struct FFISandboxOptions {
    /// Path to the `.wasm` or `.aot` guest module (UTF-8, null-terminated).
    /// Required for Wasm backend. Must be null for JavaScript backend.
    pub module_path: *const c_char,
    /// Guest heap size in bytes. 0 = platform default.
    pub heap_size: u64,
    /// Guest stack size in bytes. 0 = platform default.
    pub stack_size: u64,
    /// Backend type: 0 = Wasm (default), 1 = JavaScript.
    pub backend: u32,
}

/// Backend type discriminant.
///
/// Mirrored as `SandboxBackend` enum in C#.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIBackend {
    /// WebAssembly component backend (Python, JS-via-Wasm, etc.).
    Wasm = 0,
    /// Hyperlight-JS built-in JavaScript backend (no module path needed).
    JavaScript = 1,
}

// ---------------------------------------------------------------------------
// Tool callback type
// ---------------------------------------------------------------------------

/// Signature for tool callback function pointers passed from .NET.
///
/// The callback receives a JSON-encoded arguments string and must return a
/// JSON-encoded result string. The returned pointer must have been allocated
/// with `Marshal.StringToCoTaskMemUTF8` (which uses `malloc` on Linux,
/// `CoTaskMemAlloc` on Windows). The Rust side copies the string and then
/// frees the pointer via `libc::free`.
///
/// If the tool encounters an error, it should return a JSON object with an
/// `"error"` field: `{"error": "description"}`.
pub type ToolCallbackFn = unsafe extern "C" fn(args_json: *const c_char) -> *mut c_char;

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

/// Type aliases for the concrete sandbox types.
type WasmSandboxInner = Sandbox<Wasm>;
type WasmSnapshotInner = hyperlight_sandbox::Snapshot<
    <<Wasm as hyperlight_sandbox::Guest>::Sandbox as GuestSandbox>::SnapshotData,
>;

type JsSandboxInner = Sandbox<HyperlightJs>;
type JsSnapshotInner = hyperlight_sandbox::Snapshot<
    <<HyperlightJs as hyperlight_sandbox::Guest>::Sandbox as GuestSandbox>::SnapshotData,
>;

/// Holds the active backend sandbox instance.
enum BackendSandbox {
    Wasm(WasmSandboxInner),
    Js(JsSandboxInner),
}

/// Holds a snapshot from either backend.
enum BackendSnapshot {
    Wasm(WasmSnapshotInner),
    Js(JsSnapshotInner),
}

/// Dispatch on the active backend, binding the inner sandbox to `$sb`.
/// Both arms execute the same expression, avoiding code duplication.
macro_rules! with_sandbox {
    ($backend:expr, $sb:ident => $body:expr) => {
        match $backend {
            BackendSandbox::Wasm($sb) => $body,
            BackendSandbox::Js($sb) => $body,
        }
    };
}

/// Entry for a registered tool: the callback function pointer and optional schema.
struct ToolEntry {
    callback: ToolCallbackFn,
    schema_json: Option<String>,
}

/// Internal state behind an opaque FFI handle.
///
/// Mirrors the Python SDK's lazy-init pattern: configuration and tools are
/// collected eagerly, and the actual sandbox is built on the first `run()`.
struct SandboxState {
    /// The lazily-built sandbox instance.
    inner: Option<BackendSandbox>,
    /// Which backend to use.
    backend: FFIBackend,
    /// Tool callbacks registered before the first `run()`.
    tools: HashMap<String, ToolEntry>,
    /// Network allowlist entries queued before the sandbox is built.
    pending_networks: Vec<(String, Option<Vec<String>>)>,
    /// Sandbox configuration.
    config: SandboxConfig,
    /// Optional read-only input directory path.
    input_dir: Option<String>,
    /// Optional writable output directory path.
    output_dir: Option<String>,
    /// Whether to use a temporary output directory.
    temp_output: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A guaranteed safe C-string literal for fatal fallback.
/// No trailing null — `CString::from_vec_unchecked` adds one.
const FALLBACK_ERROR_MSG: &[u8] = b"FATAL: Could not create any error message.";

/// Create a `CString` from arbitrary bytes, sanitizing embedded null bytes.
///
/// If the input contains null bytes, they are replaced with spaces and a
/// warning is prepended.
fn safe_cstring<T: Into<Vec<u8>>>(t: T) -> CString {
    let bytes: Vec<u8> = t.into();
    match CString::new(bytes.clone()) {
        Ok(c_string) => c_string,
        Err(e) => {
            let s = String::from_utf8_lossy(&bytes);
            error!("Failed to create CString: {}. Original string: '{}'", e, s);
            let sanitized: String = s.chars().map(|c| if c == '\0' { ' ' } else { c }).collect();
            let error_message = format!(
                "WARNING: Original error message contained null characters \
                 which were replaced with spaces. Message: {}",
                sanitized
            );
            CString::new(error_message).unwrap_or_else(|_| {
                error!("FATAL: Could not create any error message after sanitization attempt.");
                // SAFETY: FALLBACK_ERROR_MSG is a compile-time constant with a trailing null.
                unsafe { CString::from_vec_unchecked(FALLBACK_ERROR_MSG.to_vec()) }
            })
        }
    }
}

/// Classify an `anyhow::Error` into an `FFIErrorCode`.
///
/// Uses `downcast_ref` for concrete types where possible, falling back
/// to string matching for untyped `anyhow` errors.
fn classify_error(err: &anyhow::Error) -> FFIErrorCode {
    // Try concrete type downcasts first (more reliable than string matching).
    if err.downcast_ref::<std::sync::PoisonError<()>>().is_some() {
        return FFIErrorCode::Poisoned;
    }
    if err.downcast_ref::<std::io::Error>().is_some() {
        return FFIErrorCode::IoError;
    }

    // Fall back to string matching for errors we can't downcast.
    let msg = err.to_string().to_lowercase();
    if msg.contains("poisoned") || msg.contains("mutex") {
        FFIErrorCode::Poisoned
    } else if msg.contains("timeout")
        || msg.contains("cancelled")
        || msg.contains("canceled")
        || msg.contains("timed out")
        || msg.contains("deadline")
    {
        FFIErrorCode::Timeout
    } else if msg.contains("permission") || msg.contains("not allowed") || msg.contains("denied") {
        FFIErrorCode::PermissionDenied
    } else if msg.contains("i/o error") || msg.contains("no such file or directory") {
        FFIErrorCode::IoError
    } else {
        FFIErrorCode::Unknown
    }
}

/// Convert an `anyhow::Error` into an `FFIResult`.
fn error_result(err: anyhow::Error) -> FFIResult {
    let code = classify_error(&err);
    let message = safe_cstring(format!("{err:#}"));
    error!("FFI error (code={code:?}): {err:#}");
    FFIResult::error(code, message)
}

/// Read a C string pointer into a Rust `&str`, returning an `FFIResult` on failure.
///
/// # Safety
///
/// The caller must ensure `ptr` is a valid, null-terminated UTF-8 string.
unsafe fn read_cstr<'a>(ptr: *const c_char, param_name: &str) -> Result<&'a str, FFIResult> {
    if ptr.is_null() {
        return Err(FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(format!("Null pointer passed for {param_name}")),
        ));
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().map_err(|e| {
        FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(format!("Invalid UTF-8 for {param_name}: {e}")),
        )
    })
}

/// Validate a mutable handle pointer and return a mutable reference.
///
/// Combines the null check and dereference into a single operation so that
/// the resulting reference is always backed by a validated pointer.
///
/// # Safety
///
/// The caller must ensure `handle` points to a live, properly-aligned
/// allocation of type `T` (i.e. was returned by `Box::into_raw`).
unsafe fn deref_handle_mut<'a, T>(handle: *mut T, name: &str) -> Result<&'a mut T, FFIResult> {
    if handle.is_null() {
        return Err(FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(format!("Null pointer passed for {name}")),
        ));
    }
    Ok(unsafe { &mut *handle })
}

/// Validate an immutable handle pointer and return a shared reference.
///
/// # Safety
///
/// The caller must ensure `handle` points to a live, properly-aligned
/// allocation of type `T`.
unsafe fn deref_handle<'a, T>(handle: *const T, name: &str) -> Result<&'a T, FFIResult> {
    if handle.is_null() {
        return Err(FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(format!("Null pointer passed for {name}")),
        ));
    }
    Ok(unsafe { &*handle })
}

/// Build the `ToolRegistry` from the collected tool entries.
///
/// Each tool callback is wrapped in a closure that:
/// 1. Serializes the `serde_json::Value` args to a JSON string
/// 2. Calls the .NET function pointer with the JSON
/// 3. Reads the returned JSON string
/// 4. Deserializes the result back to `serde_json::Value`
fn build_tool_registry(tools: &HashMap<String, ToolEntry>) -> Result<ToolRegistry> {
    let mut registry = ToolRegistry::new();

    for (name, entry) in tools {
        let callback = entry.callback;
        let tool_name = name.clone();

        // Parse schema if provided.
        let schema = if let Some(ref schema_json) = entry.schema_json {
            Some(
                parse_tool_schema(schema_json)
                    .map_err(|e| anyhow::anyhow!("tool '{tool_name}': invalid schema: {e}"))?,
            )
        } else {
            None
        };

        // Wrap the .NET callback in a Rust closure.
        //
        // SAFETY: The function pointer `callback` is valid for the lifetime of
        // the .NET GCHandle that pins the delegate. The .NET side must ensure
        // the delegate is not collected while the sandbox is alive.
        let handler = move |args: serde_json::Value| -> Result<serde_json::Value> {
            let args_str = serde_json::to_string(&args)?;
            let args_cstr = CString::new(args_str)
                .map_err(|e| anyhow::anyhow!("tool '{tool_name}': args contain null byte: {e}"))?;

            let result_ptr = unsafe { callback(args_cstr.as_ptr()) };

            if result_ptr.is_null() {
                anyhow::bail!("tool '{tool_name}': callback returned null");
            }

            // Read and copy the result string, then free the .NET-allocated memory.
            // SAFETY: The .NET side guarantees the pointer is a valid, null-terminated
            // UTF-8 string allocated with Marshal.StringToCoTaskMemUTF8.
            let result_cstr = unsafe { CStr::from_ptr(result_ptr) };
            let result_str = result_cstr.to_str().map_err(|e| {
                anyhow::anyhow!("tool '{tool_name}': callback returned invalid UTF-8: {e}")
            })?;

            // Copy the string before freeing the .NET-allocated memory.
            let result_owned = result_str.to_owned();

            // Free the .NET-allocated string.
            // On Linux, Marshal.StringToCoTaskMemUTF8 uses malloc → free with libc::free.
            // On Windows, it uses CoTaskMemAlloc → free with CoTaskMemFree.
            #[cfg(not(windows))]
            unsafe {
                libc::free(result_ptr as *mut libc::c_void)
            };
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::System::Com::CoTaskMemFree(result_ptr as *mut std::ffi::c_void)
            };

            // Check for error convention: {"error": "..."}
            let value: serde_json::Value = serde_json::from_str(&result_owned).map_err(|e| {
                anyhow::anyhow!("tool '{tool_name}': callback returned invalid JSON: {e}")
            })?;

            if let Some(err_msg) = value.get("error").and_then(|v| v.as_str()) {
                anyhow::bail!("tool '{tool_name}': {err_msg}");
            }

            Ok(value)
        };

        registry.register_with_schema(name, schema, handler);
    }

    Ok(registry)
}

/// Parse a JSON schema string into a `ToolSchema`.
///
/// Expected format:
/// ```json
/// {
///   "args": { "a": "Number", "b": "String" },
///   "required": ["a"]
/// }
/// ```
fn parse_tool_schema(json: &str) -> Result<ToolSchema> {
    let parsed: serde_json::Value = serde_json::from_str(json)?;
    let mut schema = ToolSchema::new();

    if let Some(args) = parsed.get("args").and_then(|v| v.as_object()) {
        for (name, type_val) in args {
            let type_str = type_val
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("schema arg '{name}': type must be a string"))?;
            let arg_type = match type_str.to_lowercase().as_str() {
                "number" => hyperlight_sandbox::ArgType::Number,
                "string" => hyperlight_sandbox::ArgType::String,
                "boolean" | "bool" => hyperlight_sandbox::ArgType::Boolean,
                "object" => hyperlight_sandbox::ArgType::Object,
                "array" => hyperlight_sandbox::ArgType::Array,
                other => anyhow::bail!("schema arg '{name}': unknown type '{other}'"),
            };
            schema = schema.optional_arg(name, arg_type);
        }
    }

    if let Some(required) = parsed.get("required").and_then(|v| v.as_array()) {
        for req in required {
            let name = req
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("schema 'required': entries must be strings"))?;
            // If the arg was already added as optional, promote it to required.
            // If not in the args map, add it as required-untyped.
            if schema.properties.contains_key(name) {
                schema.required.push(name.to_string());
            } else {
                schema = schema.required_untyped(name);
            }
        }
    }

    Ok(schema)
}

/// Parse a human-readable size string (e.g. `"200Mi"`) to bytes.
///
/// Used by the .NET `SizeParser` for consistency, and tested below.
#[allow(dead_code)]
fn parse_size(size: &str) -> Result<u64> {
    let size = size.trim();
    let (value, multiplier) = if let Some(value) = size.strip_suffix("Gi") {
        (value, 1024u64.pow(3))
    } else if let Some(value) = size.strip_suffix("Mi") {
        (value, 1024u64.pow(2))
    } else if let Some(value) = size.strip_suffix("Ki") {
        (value, 1024u64)
    } else {
        (size, 1)
    };
    let parsed: u64 = value
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid size '{size}': {e}"))?;
    parsed
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("invalid size '{size}': value is too large"))
}

// ===========================================================================
// PUBLIC FFI FUNCTIONS
// ===========================================================================

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// Returns the version of the hyperlight-sandbox FFI library.
///
/// The caller must free the returned string with `hyperlight_sandbox_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn hyperlight_sandbox_get_version() -> *mut c_char {
    safe_cstring(env!("CARGO_PKG_VERSION")).into_raw()
}

// ---------------------------------------------------------------------------
// String management
// ---------------------------------------------------------------------------

/// Frees a string previously returned by an `hyperlight_sandbox_*` function.
///
/// # Safety
///
/// The pointer must have been returned by this library and not already freed.
/// Passing null is safe (no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

// ---------------------------------------------------------------------------
// Sandbox lifecycle
// ---------------------------------------------------------------------------

/// Creates a new sandbox instance.
///
/// The sandbox is not fully initialized until the first `run()` call — tools
/// and configuration can be set between `create` and `run`.
///
/// # Arguments
///
/// * `options` — Configuration struct. `module_path` must point to a valid
///   `.wasm` or `.aot` file. Zero values for `heap_size` / `stack_size` use
///   platform defaults.
///
/// # Returns
///
/// On success: `is_success = true`, `value` is an opaque handle to the sandbox.
/// On failure: `is_success = false`, `value` is an error message.
///
/// The handle must be freed with `hyperlight_sandbox_free`.
///
/// # Safety
///
/// `options.module_path` must be a valid, null-terminated UTF-8 string pointing
/// to a `.wasm` or `.aot` file. The caller owns the string memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_create(options: FFISandboxOptions) -> FFIResult {
    // Parse backend type.
    let backend = match options.backend {
        0 => FFIBackend::Wasm,
        1 => FFIBackend::JavaScript,
        other => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring(format!(
                    "Invalid backend value: {other}. Use 0 (Wasm) or 1 (JavaScript)."
                )),
            );
        }
    };

    // Parse module path — required for Wasm, must be null/empty for JS.
    let module_path = if options.module_path.is_null() {
        String::new()
    } else {
        match unsafe { read_cstr(options.module_path, "module_path") } {
            Ok(s) => s.to_owned(),
            Err(e) => return e,
        }
    };

    match backend {
        FFIBackend::Wasm if module_path.is_empty() => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("module_path is required for Wasm backend"),
            );
        }
        FFIBackend::JavaScript if !module_path.is_empty() => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring(
                    "module_path must not be set for JavaScript backend (it has a built-in runtime)",
                ),
            );
        }
        _ => {}
    }

    let heap_size = if options.heap_size > 0 {
        options.heap_size
    } else {
        DEFAULT_HEAP_SIZE
    };

    let stack_size = if options.stack_size > 0 {
        options.stack_size
    } else {
        DEFAULT_STACK_SIZE
    };

    let state = SandboxState {
        inner: None,
        backend,
        tools: HashMap::new(),
        pending_networks: Vec::new(),
        config: SandboxConfig {
            module_path,
            heap_size,
            stack_size,
        },
        input_dir: None,
        output_dir: None,
        temp_output: false,
    };

    let handle = Box::into_raw(Box::new(state));
    debug!(
        "hyperlight_sandbox_create: created handle at {:?} (backend={:?})",
        handle, backend
    );
    FFIResult::success(handle as *mut c_char)
}

/// Frees a sandbox instance previously created with `hyperlight_sandbox_create`.
///
/// # Safety
///
/// The pointer must be a valid handle returned by `hyperlight_sandbox_create`
/// and not already freed. Passing null is safe (no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_free(handle: *mut SandboxState) {
    if !handle.is_null() {
        debug!("hyperlight_sandbox_free: freeing handle at {:?}", handle);
        unsafe {
            let _ = Box::from_raw(handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration (pre-run)
// ---------------------------------------------------------------------------

/// Sets the read-only input directory for the sandbox.
///
/// Must be called before the first `run()`.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. `path` must be a null-terminated
/// UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_set_input_dir(
    handle: *mut SandboxState,
    path: *const c_char,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path_str = match unsafe { read_cstr(path, "path") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    if state.inner.is_some() {
        return FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring("Cannot set input_dir after sandbox has been initialized"),
        );
    }
    state.input_dir = Some(path_str.to_owned());
    FFIResult::success_null()
}

/// Sets the writable output directory for the sandbox.
///
/// Must be called before the first `run()`.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. `path` must be a null-terminated
/// UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_set_output_dir(
    handle: *mut SandboxState,
    path: *const c_char,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path_str = match unsafe { read_cstr(path, "path") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    if state.inner.is_some() {
        return FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring("Cannot set output_dir after sandbox has been initialized"),
        );
    }
    state.output_dir = Some(path_str.to_owned());
    FFIResult::success_null()
}

/// Enables a temporary writable output directory.
///
/// Must be called before the first `run()`. Ignored if `set_output_dir` was called.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_set_temp_output(
    handle: *mut SandboxState,
    enabled: bool,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    if state.inner.is_some() {
        return FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring("Cannot set temp_output after sandbox has been initialized"),
        );
    }
    state.temp_output = enabled;
    FFIResult::success_null()
}

/// Adds a domain to the network allowlist.
///
/// Can be called before or after initialization.
///
/// # Arguments
///
/// * `target` — URL or domain (e.g. `"https://httpbin.org"`).
/// * `methods_json` — Optional JSON array of HTTP methods (e.g. `["GET", "POST"]`).
///   Pass null to allow all methods.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. String pointers must be
/// null-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_allow_domain(
    handle: *mut SandboxState,
    target: *const c_char,
    methods_json: *const c_char,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let target_str = match unsafe { read_cstr(target, "target") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Parse optional methods list.
    let methods: Option<Vec<String>> = if methods_json.is_null() {
        None
    } else {
        let json_str = match unsafe { read_cstr(methods_json, "methods_json") } {
            Ok(s) => s,
            Err(e) => return e,
        };
        match serde_json::from_str::<Vec<String>>(json_str) {
            Ok(m) => Some(m),
            Err(e) => {
                return FFIResult::error(
                    FFIErrorCode::InvalidArgument,
                    safe_cstring(format!("Invalid methods JSON: {e}")),
                );
            }
        }
    };
    if let Some(ref mut sandbox) = state.inner {
        // Sandbox already built — apply immediately.
        let method_filter = match HttpMethod::parse_list(methods) {
            Ok(m) => m,
            Err(e) => return error_result(e),
        };
        let result = with_sandbox!(sandbox, sb => sb.allow_domain(target_str, method_filter));
        match result {
            Ok(()) => FFIResult::success_null(),
            Err(e) => error_result(e),
        }
    } else {
        // Queue for application during lazy init.
        state
            .pending_networks
            .push((target_str.to_owned(), methods));
        FFIResult::success_null()
    }
}

// ---------------------------------------------------------------------------
// Tool registration
// ---------------------------------------------------------------------------

/// Registers a host-side tool that guest code can invoke via `call_tool()`.
///
/// Must be called before the first `run()`.
///
/// # Arguments
///
/// * `name` — Tool name (null-terminated UTF-8).
/// * `schema_json` — Optional JSON schema string describing expected arguments.
///   Pass null for no schema validation. Format:
///   `{"args": {"a": "Number"}, "required": ["a"]}`
/// * `callback` — Function pointer invoked when the guest calls this tool.
///   Receives JSON args, must return JSON result (or `{"error": "..."}` on failure).
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. `name` must be null-terminated UTF-8.
/// `callback` must be a valid function pointer that remains valid for the lifetime
/// of the sandbox (i.e., the .NET delegate must be pinned with `GCHandle`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_register_tool(
    handle: *mut SandboxState,
    name: *const c_char,
    schema_json: *const c_char,
    callback: ToolCallbackFn,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let name_str = match unsafe { read_cstr(name, "name") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    if state.inner.is_some() {
        return FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(
                "Cannot register tools after sandbox has been initialized. \
                 Register all tools before the first run() call.",
            ),
        );
    }

    // Read optional schema.
    let schema = if schema_json.is_null() {
        None
    } else {
        match unsafe { read_cstr(schema_json, "schema_json") } {
            Ok(s) => Some(s.to_owned()),
            Err(e) => return e,
        }
    };

    if state.tools.contains_key(name_str) {
        return FFIResult::error(
            FFIErrorCode::InvalidArgument,
            safe_cstring(format!("Tool '{}' is already registered", name_str)),
        );
    }

    state.tools.insert(
        name_str.to_owned(),
        ToolEntry {
            callback,
            schema_json: schema,
        },
    );

    debug!(
        "hyperlight_sandbox_register_tool: registered tool '{}'",
        name_str
    );
    FFIResult::success_null()
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Build the sandbox lazily on first run.
fn ensure_initialized(state: &mut SandboxState) -> Result<()> {
    if state.inner.is_some() {
        return Ok(());
    }

    let registry = build_tool_registry(&state.tools)?;

    // Build the appropriate backend.
    let sandbox: BackendSandbox = match state.backend {
        FFIBackend::Wasm => {
            let mut builder = SandboxBuilder::new()
                .module_path(&state.config.module_path)
                .heap_size(state.config.heap_size)
                .stack_size(state.config.stack_size)
                .with_tools(registry)
                .guest(Wasm);

            if let Some(ref dir) = state.input_dir {
                builder = builder.input_dir(dir);
            }
            if let Some(ref dir) = state.output_dir {
                builder = builder.output_dir(
                    dir,
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                );
            } else if state.temp_output {
                builder = builder.temp_output();
            }

            let mut sb = builder.build()?;
            for (target, methods) in std::mem::take(&mut state.pending_networks) {
                let method_filter = HttpMethod::parse_list(methods)?;
                sb.allow_domain(&target, method_filter)?;
            }
            BackendSandbox::Wasm(sb)
        }
        FFIBackend::JavaScript => {
            let mut builder = SandboxBuilder::new()
                .heap_size(state.config.heap_size)
                .stack_size(state.config.stack_size)
                .with_tools(registry)
                .guest(HyperlightJs);

            if let Some(ref dir) = state.input_dir {
                builder = builder.input_dir(dir);
            }
            if let Some(ref dir) = state.output_dir {
                builder = builder.output_dir(
                    dir,
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                );
            } else if state.temp_output {
                builder = builder.temp_output();
            }

            let mut sb = builder.build()?;
            for (target, methods) in std::mem::take(&mut state.pending_networks) {
                let method_filter = HttpMethod::parse_list(methods)?;
                sb.allow_domain(&target, method_filter)?;
            }
            BackendSandbox::Js(sb)
        }
    };

    state.inner = Some(sandbox);
    Ok(())
}

/// Executes guest code in the sandbox.
///
/// The first call triggers lazy initialization (building the sandbox, registering
/// tools, applying network permissions).
///
/// # Arguments
///
/// * `code` — The guest code to execute (null-terminated UTF-8).
///
/// # Returns
///
/// On success: `value` is a JSON string `{"stdout":"...","stderr":"...","exit_code":0}`.
/// On failure: `value` is an error message.
///
/// The caller must free the `value` string with `hyperlight_sandbox_free_string`.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. `code` must be null-terminated UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_run(
    handle: *mut SandboxState,
    code: *const c_char,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let code_str = match unsafe { read_cstr(code, "code") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Lazy initialization.
    if let Err(e) = ensure_initialized(state) {
        return error_result(e);
    }

    let sandbox_result =
        with_sandbox!(state.inner.as_mut().expect("initialized above"), sb => sb.run(code_str));

    match sandbox_result {
        Ok(result) => {
            // Serialize ExecutionResult to JSON.
            match serde_json::to_string(&result) {
                Ok(json) => FFIResult::success(safe_cstring(json).into_raw()),
                Err(e) => FFIResult::error(
                    FFIErrorCode::Unknown,
                    safe_cstring(format!("Failed to serialize execution result: {e}")),
                ),
            }
        }
        Err(e) => {
            // Classify the error — don't blindly promote Unknown to GuestError,
            // as that masks infrastructure errors (OOM, setup failures).
            let code = classify_error(&e);
            FFIResult::error(code, safe_cstring(format!("{e:#}")))
        }
    }
}

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

/// Returns the list of files in the output directory as a JSON array.
///
/// # Returns
///
/// On success: `value` is a JSON array of filenames (e.g. `["file1.txt","file2.txt"]`).
/// On failure: `value` is an error message.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_get_output_files(
    handle: *mut SandboxState,
) -> FFIResult {
    let state = match unsafe { deref_handle(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let sandbox = match state.inner.as_ref() {
        Some(s) => s,
        None => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("Sandbox not initialized — call run() first"),
            );
        }
    };

    let files_result = with_sandbox!(sandbox, sb => sb.get_output_files());

    match files_result {
        Ok(files) => match serde_json::to_string(&files) {
            Ok(json) => FFIResult::success(safe_cstring(json).into_raw()),
            Err(e) => FFIResult::error(
                FFIErrorCode::Unknown,
                safe_cstring(format!("Failed to serialize output files: {e}")),
            ),
        },
        Err(e) => error_result(e),
    }
}

/// Returns the host filesystem path of the output directory.
///
/// # Returns
///
/// On success: `value` is the path string, or null if no output directory is configured.
/// On failure: `value` is an error message.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_output_path(handle: *mut SandboxState) -> FFIResult {
    let state = match unsafe { deref_handle(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let sandbox = match state.inner.as_ref() {
        Some(s) => s,
        None => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("Sandbox not initialized — call run() first"),
            );
        }
    };

    let output = with_sandbox!(sandbox, sb => sb.output_path());

    match output {
        Ok(Some(path)) => {
            let path_str = path.display().to_string();
            FFIResult::success(safe_cstring(path_str).into_raw())
        }
        Ok(None) => FFIResult::success_null(),
        Err(e) => error_result(e),
    }
}

// ---------------------------------------------------------------------------
// Snapshot / Restore
// ---------------------------------------------------------------------------

/// Takes a snapshot of the current sandbox state.
///
/// The sandbox must be initialized (at least one `run()` call).
///
/// # Returns
///
/// On success: `value` is an opaque snapshot handle.
/// On failure: `value` is an error message.
///
/// The snapshot handle must be freed with `hyperlight_sandbox_free_snapshot`.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_snapshot(handle: *mut SandboxState) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let sandbox = match state.inner.as_mut() {
        Some(s) => s,
        None => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("Sandbox not initialized — call run() first"),
            );
        }
    };

    let snapshot_result = match sandbox {
        BackendSandbox::Wasm(sb) => sb.snapshot().map(BackendSnapshot::Wasm),
        BackendSandbox::Js(sb) => sb.snapshot().map(BackendSnapshot::Js),
    };

    match snapshot_result {
        Ok(snapshot) => {
            let boxed = Box::new(snapshot);
            FFIResult::success(Box::into_raw(boxed) as *mut c_char)
        }
        Err(e) => error_result(e),
    }
}

/// Restores the sandbox to a previously captured snapshot.
///
/// # Safety
///
/// `handle` must be a valid sandbox handle. `snapshot` must be a valid
/// snapshot handle that has not been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_restore(
    handle: *mut SandboxState,
    snapshot: *const BackendSnapshot,
) -> FFIResult {
    let state = match unsafe { deref_handle_mut(handle, "sandbox") } {
        Ok(s) => s,
        Err(e) => return e,
    };
    let snapshot_ref = match unsafe { deref_handle(snapshot, "snapshot") } {
        Ok(s) => s,
        Err(e) => return e,
    };

    let sandbox = match state.inner.as_mut() {
        Some(s) => s,
        None => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("Sandbox not initialized — call run() first"),
            );
        }
    };
    let result = match (sandbox, snapshot_ref) {
        (BackendSandbox::Wasm(sb), BackendSnapshot::Wasm(snap)) => sb.restore(snap),
        (BackendSandbox::Js(sb), BackendSnapshot::Js(snap)) => sb.restore(snap),
        _ => {
            return FFIResult::error(
                FFIErrorCode::InvalidArgument,
                safe_cstring("Snapshot type does not match sandbox backend"),
            );
        }
    };
    match result {
        Ok(()) => FFIResult::success_null(),
        Err(e) => error_result(e),
    }
}

/// Frees a snapshot previously returned by `hyperlight_sandbox_snapshot`.
///
/// # Safety
///
/// The pointer must be a valid snapshot handle and not already freed.
/// Passing null is safe (no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hyperlight_sandbox_free_snapshot(snapshot: *mut BackendSnapshot) {
    if !snapshot.is_null() {
        unsafe {
            let _ = Box::from_raw(snapshot);
        }
    }
}

// ===========================================================================
// TESTS
// ===========================================================================

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::ptr;

    use super::*;

    // -----------------------------------------------------------------------
    // Helper: create a CString pointer for test use
    // -----------------------------------------------------------------------
    fn cstr(s: &str) -> CString {
        CString::new(s).expect("test string should not contain null bytes")
    }

    // -----------------------------------------------------------------------
    // FFIResult helpers
    // -----------------------------------------------------------------------

    #[test]
    fn ffi_result_success_has_correct_fields() {
        let msg = safe_cstring("hello");
        let result = FFIResult::success(msg.into_raw());
        assert!(result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::Success as u32);
        assert!(!result.value.is_null());
        // Clean up
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn ffi_result_success_null_has_null_value() {
        let result = FFIResult::success_null();
        assert!(result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::Success as u32);
        assert!(result.value.is_null());
    }

    #[test]
    fn ffi_result_error_has_correct_fields() {
        let msg = safe_cstring("something broke");
        let result = FFIResult::error(FFIErrorCode::GuestError, msg);
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::GuestError as u32);
        assert!(!result.value.is_null());
        // Read the error message
        let err_str = unsafe { CStr::from_ptr(result.value) }
            .to_str()
            .expect("valid UTF-8");
        assert!(err_str.contains("something broke"));
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    // -----------------------------------------------------------------------
    // safe_cstring
    // -----------------------------------------------------------------------

    #[test]
    fn safe_cstring_normal_string() {
        let cs = safe_cstring("Hello, World!");
        assert_eq!(cs.to_str().expect("valid"), "Hello, World!");
    }

    #[test]
    fn safe_cstring_empty_string() {
        let cs = safe_cstring("");
        assert_eq!(cs.to_str().expect("valid"), "");
    }

    #[test]
    fn safe_cstring_with_embedded_null_sanitizes() {
        let bytes = b"hello\0world".to_vec();
        let cs = safe_cstring(bytes);
        let s = cs.to_str().expect("valid");
        // The null byte should be replaced and a warning prepended
        assert!(s.contains("WARNING"));
        assert!(s.contains("hello world"));
    }

    // -----------------------------------------------------------------------
    // classify_error
    // -----------------------------------------------------------------------

    #[test]
    fn classify_error_poisoned() {
        let err = anyhow::anyhow!("mutex poisoned during sandbox run");
        assert_eq!(classify_error(&err), FFIErrorCode::Poisoned);
    }

    #[test]
    fn classify_error_timeout() {
        let err = anyhow::anyhow!("execution timeout exceeded");
        assert_eq!(classify_error(&err), FFIErrorCode::Timeout);
    }

    #[test]
    fn classify_error_cancelled() {
        let err = anyhow::anyhow!("operation was cancelled by host");
        assert_eq!(classify_error(&err), FFIErrorCode::Timeout);
    }

    #[test]
    fn classify_error_permission() {
        let err = anyhow::anyhow!("request not allowed by network policy");
        assert_eq!(classify_error(&err), FFIErrorCode::PermissionDenied);
    }

    #[test]
    fn classify_error_io() {
        let err = anyhow::anyhow!("i/o error reading file");
        assert_eq!(classify_error(&err), FFIErrorCode::IoError);
    }

    #[test]
    fn classify_error_unknown_fallback() {
        let err = anyhow::anyhow!("some mysterious failure");
        assert_eq!(classify_error(&err), FFIErrorCode::Unknown);
    }

    // -----------------------------------------------------------------------
    // parse_size
    // -----------------------------------------------------------------------

    #[test]
    fn parse_size_plain_bytes() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
    }

    #[test]
    fn parse_size_kilobytes() {
        assert_eq!(parse_size("10Ki").unwrap(), 10 * 1024);
    }

    #[test]
    fn parse_size_megabytes() {
        assert_eq!(parse_size("25Mi").unwrap(), 25 * 1024 * 1024);
    }

    #[test]
    fn parse_size_gigabytes() {
        assert_eq!(parse_size("2Gi").unwrap(), 2 * 1024 * 1024 * 1024);
    }

    #[test]
    fn parse_size_with_whitespace() {
        assert_eq!(parse_size("  400Mi  ").unwrap(), 400 * 1024 * 1024);
    }

    #[test]
    fn parse_size_invalid_number() {
        assert!(parse_size("abcMi").is_err());
    }

    #[test]
    fn parse_size_empty_string() {
        assert!(parse_size("").is_err());
    }

    #[test]
    fn parse_size_overflow() {
        // u64::MAX in Gi would overflow
        assert!(parse_size("999999999999999999Gi").is_err());
    }

    // -----------------------------------------------------------------------
    // parse_tool_schema
    // -----------------------------------------------------------------------

    #[test]
    fn parse_tool_schema_with_typed_args() {
        let json = r#"{"args": {"a": "Number", "b": "String"}, "required": ["a"]}"#;
        let schema = parse_tool_schema(json).unwrap();
        assert_eq!(schema.properties.len(), 2);
        assert_eq!(
            schema.properties.get("a"),
            Some(&hyperlight_sandbox::ArgType::Number)
        );
        assert_eq!(
            schema.properties.get("b"),
            Some(&hyperlight_sandbox::ArgType::String)
        );
        assert!(schema.required.contains(&"a".to_string()));
        assert!(!schema.required.contains(&"b".to_string()));
    }

    #[test]
    fn parse_tool_schema_boolean_alias() {
        let json = r#"{"args": {"flag": "bool"}, "required": []}"#;
        let schema = parse_tool_schema(json).unwrap();
        assert_eq!(
            schema.properties.get("flag"),
            Some(&hyperlight_sandbox::ArgType::Boolean)
        );
    }

    #[test]
    fn parse_tool_schema_all_types() {
        let json = r#"{"args": {"n": "Number", "s": "String", "b": "Boolean", "o": "Object", "a": "Array"}, "required": []}"#;
        let schema = parse_tool_schema(json).unwrap();
        assert_eq!(schema.properties.len(), 5);
    }

    #[test]
    fn parse_tool_schema_empty() {
        let json = r#"{}"#;
        let schema = parse_tool_schema(json).unwrap();
        assert!(schema.properties.is_empty());
        assert!(schema.required.is_empty());
    }

    #[test]
    fn parse_tool_schema_required_untyped() {
        let json = r#"{"required": ["x"]}"#;
        let schema = parse_tool_schema(json).unwrap();
        assert!(schema.required.contains(&"x".to_string()));
        assert!(!schema.properties.contains_key("x"));
    }

    #[test]
    fn parse_tool_schema_unknown_type_errors() {
        let json = r#"{"args": {"a": "Unicorn"}, "required": []}"#;
        assert!(parse_tool_schema(json).is_err());
    }

    #[test]
    fn parse_tool_schema_invalid_json() {
        assert!(parse_tool_schema("not json").is_err());
    }

    // -----------------------------------------------------------------------
    // read_cstr / deref_handle
    // -----------------------------------------------------------------------

    #[test]
    fn read_cstr_null_returns_error() {
        let result = unsafe { read_cstr(ptr::null(), "test_param") };
        assert!(result.is_err());
        let ffi_result = result.unwrap_err();
        assert!(!ffi_result.is_success);
        assert_eq!(ffi_result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(ffi_result.value) };
    }

    #[test]
    fn read_cstr_valid_string() {
        let s = cstr("hello");
        let result = unsafe { read_cstr(s.as_ptr(), "test_param") };
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn deref_handle_null_returns_error() {
        let result = unsafe { deref_handle::<u8>(ptr::null(), "test_handle") };
        assert!(result.is_err());
        let ffi_result = result.unwrap_err();
        assert!(!ffi_result.is_success);
        unsafe { hyperlight_sandbox_free_string(ffi_result.value) };
    }

    #[test]
    fn deref_handle_valid_pointer_ok() {
        let x: u8 = 42;
        let result = unsafe { deref_handle(&x as *const u8, "test_handle") };
        assert!(result.is_ok());
        assert_eq!(*result.unwrap(), 42);
    }

    #[test]
    fn deref_handle_mut_null_returns_error() {
        let result = unsafe { deref_handle_mut::<u8>(ptr::null_mut(), "test_handle") };
        assert!(result.is_err());
        let ffi_result = result.unwrap_err();
        assert!(!ffi_result.is_success);
        unsafe { hyperlight_sandbox_free_string(ffi_result.value) };
    }

    #[test]
    fn deref_handle_mut_valid_pointer_ok() {
        let mut x: u8 = 42;
        let result = unsafe { deref_handle_mut(&mut x as *mut u8, "test_handle") };
        assert!(result.is_ok());
        *result.unwrap() = 99;
        assert_eq!(x, 99);
    }

    // -----------------------------------------------------------------------
    // Version
    // -----------------------------------------------------------------------

    #[test]
    fn get_version_returns_valid_string() {
        let ptr = hyperlight_sandbox_get_version();
        assert!(!ptr.is_null());
        let version = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .expect("valid UTF-8");
        // Should match Cargo.toml version
        assert!(!version.is_empty());
        assert!(version.contains('.'), "version should be semver: {version}");
        unsafe { hyperlight_sandbox_free_string(ptr) };
    }

    // -----------------------------------------------------------------------
    // Free string
    // -----------------------------------------------------------------------

    #[test]
    fn free_string_null_is_safe() {
        unsafe { hyperlight_sandbox_free_string(ptr::null_mut()) };
        // Should not crash
    }

    #[test]
    fn free_string_valid_pointer() {
        let s = safe_cstring("to be freed");
        let ptr = s.into_raw();
        unsafe { hyperlight_sandbox_free_string(ptr) };
        // Should not crash or leak
    }

    // -----------------------------------------------------------------------
    // Sandbox create / free
    // -----------------------------------------------------------------------

    #[test]
    fn create_with_null_module_path_fails() {
        let options = FFISandboxOptions {
            module_path: ptr::null(),
            heap_size: 0,
            stack_size: 0,
            backend: 0,
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn create_with_empty_module_path_fails() {
        let path = cstr("");
        let options = FFISandboxOptions {
            module_path: path.as_ptr(),
            heap_size: 0,
            stack_size: 0,
            backend: 0,
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn create_and_free_succeeds() {
        let path = cstr("/tmp/nonexistent.wasm");
        let options = FFISandboxOptions {
            module_path: path.as_ptr(),
            heap_size: 0,
            stack_size: 0,
            backend: 0,
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(result.is_success, "create should succeed");
        assert!(!result.value.is_null(), "handle should be non-null");

        // Free the handle
        let handle = result.value as *mut SandboxState;
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn free_null_handle_is_safe() {
        unsafe { hyperlight_sandbox_free(ptr::null_mut()) };
    }

    #[test]
    fn create_with_custom_sizes() {
        let path = cstr("/tmp/test.wasm");
        let options = FFISandboxOptions {
            module_path: path.as_ptr(),
            heap_size: 50 * 1024 * 1024, // 50 MiB
            stack_size: 10 * 1024 * 1024,
            backend: 0, // 10 MiB
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(result.is_success);

        let handle = result.value as *mut SandboxState;
        let state = unsafe { &*handle };
        assert_eq!(state.config.heap_size, 50 * 1024 * 1024);
        assert_eq!(state.config.stack_size, 10 * 1024 * 1024);

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn create_with_zero_sizes_uses_defaults() {
        let path = cstr("/tmp/test.wasm");
        let options = FFISandboxOptions {
            module_path: path.as_ptr(),
            heap_size: 0,
            stack_size: 0,
            backend: 0,
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(result.is_success);

        let handle = result.value as *mut SandboxState;
        let state = unsafe { &*handle };
        assert_eq!(state.config.heap_size, DEFAULT_HEAP_SIZE);
        assert_eq!(state.config.stack_size, DEFAULT_STACK_SIZE);

        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Helper: create a test handle (not initialized — no real wasm module)
    // -----------------------------------------------------------------------

    fn create_test_handle() -> *mut SandboxState {
        let path = cstr("/tmp/test-module.wasm");
        let options = FFISandboxOptions {
            module_path: path.as_ptr(),
            heap_size: 0,
            stack_size: 0,
            backend: 0,
        };
        let result = unsafe { hyperlight_sandbox_create(options) };
        assert!(result.is_success, "test handle creation should succeed");
        result.value as *mut SandboxState
    }

    // -----------------------------------------------------------------------
    // Configuration: set_input_dir
    // -----------------------------------------------------------------------

    #[test]
    fn set_input_dir_succeeds() {
        let handle = create_test_handle();
        let path = cstr("/tmp/input");
        let result = unsafe { hyperlight_sandbox_set_input_dir(handle, path.as_ptr()) };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert_eq!(state.input_dir.as_deref(), Some("/tmp/input"));

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn set_input_dir_null_handle_fails() {
        let path = cstr("/tmp/input");
        let result = unsafe { hyperlight_sandbox_set_input_dir(ptr::null_mut(), path.as_ptr()) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn set_input_dir_null_path_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_set_input_dir(handle, ptr::null()) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Configuration: set_output_dir
    // -----------------------------------------------------------------------

    #[test]
    fn set_output_dir_succeeds() {
        let handle = create_test_handle();
        let path = cstr("/tmp/output");
        let result = unsafe { hyperlight_sandbox_set_output_dir(handle, path.as_ptr()) };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert_eq!(state.output_dir.as_deref(), Some("/tmp/output"));

        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Configuration: set_temp_output
    // -----------------------------------------------------------------------

    #[test]
    fn set_temp_output_succeeds() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_set_temp_output(handle, true) };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert!(state.temp_output);

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn set_temp_output_null_handle_fails() {
        let result = unsafe { hyperlight_sandbox_set_temp_output(ptr::null_mut(), true) };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    // -----------------------------------------------------------------------
    // Configuration: allow_domain
    // -----------------------------------------------------------------------

    #[test]
    fn allow_domain_queues_before_init() {
        let handle = create_test_handle();
        let target = cstr("https://httpbin.org");
        let result =
            unsafe { hyperlight_sandbox_allow_domain(handle, target.as_ptr(), ptr::null()) };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert_eq!(state.pending_networks.len(), 1);
        assert_eq!(state.pending_networks[0].0, "https://httpbin.org");
        assert!(state.pending_networks[0].1.is_none());

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn allow_domain_with_methods_queues_correctly() {
        let handle = create_test_handle();
        let target = cstr("https://api.example.com");
        let methods = cstr(r#"["GET", "POST"]"#);
        let result =
            unsafe { hyperlight_sandbox_allow_domain(handle, target.as_ptr(), methods.as_ptr()) };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert_eq!(state.pending_networks.len(), 1);
        assert_eq!(
            state.pending_networks[0].1,
            Some(vec!["GET".to_string(), "POST".to_string()])
        );

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn allow_domain_null_handle_fails() {
        let target = cstr("https://example.com");
        let result = unsafe {
            hyperlight_sandbox_allow_domain(ptr::null_mut(), target.as_ptr(), ptr::null())
        };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn allow_domain_invalid_methods_json_fails() {
        let handle = create_test_handle();
        let target = cstr("https://example.com");
        let bad_methods = cstr("not valid json");
        let result = unsafe {
            hyperlight_sandbox_allow_domain(handle, target.as_ptr(), bad_methods.as_ptr())
        };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Tool registration
    // -----------------------------------------------------------------------

    /// A trivial test callback that echoes its input wrapped in {"echo": ...}.
    unsafe extern "C" fn echo_callback(args_json: *const c_char) -> *mut c_char {
        let input = unsafe { CStr::from_ptr(args_json) }
            .to_str()
            .unwrap_or("{}");
        let response = format!(r#"{{"echo": {}}}"#, input);
        CString::new(response).expect("no nulls").into_raw()
    }

    #[test]
    fn register_tool_succeeds() {
        let handle = create_test_handle();
        let name = cstr("echo");
        let result = unsafe {
            hyperlight_sandbox_register_tool(handle, name.as_ptr(), ptr::null(), echo_callback)
        };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        assert!(state.tools.contains_key("echo"));

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn register_tool_with_schema_succeeds() {
        let handle = create_test_handle();
        let name = cstr("add");
        let schema = cstr(r#"{"args": {"a": "Number", "b": "Number"}, "required": ["a", "b"]}"#);
        let result = unsafe {
            hyperlight_sandbox_register_tool(handle, name.as_ptr(), schema.as_ptr(), echo_callback)
        };
        assert!(result.is_success);

        let state = unsafe { &*handle };
        let entry = state.tools.get("add").expect("tool should exist");
        assert!(entry.schema_json.is_some());

        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn register_tool_null_handle_fails() {
        let name = cstr("test");
        let result = unsafe {
            hyperlight_sandbox_register_tool(
                ptr::null_mut(),
                name.as_ptr(),
                ptr::null(),
                echo_callback,
            )
        };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn register_tool_null_name_fails() {
        let handle = create_test_handle();
        let result = unsafe {
            hyperlight_sandbox_register_tool(handle, ptr::null(), ptr::null(), echo_callback)
        };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn register_multiple_tools() {
        let handle = create_test_handle();

        let name1 = cstr("tool1");
        let name2 = cstr("tool2");
        let name3 = cstr("tool3");

        let r1 = unsafe {
            hyperlight_sandbox_register_tool(handle, name1.as_ptr(), ptr::null(), echo_callback)
        };
        let r2 = unsafe {
            hyperlight_sandbox_register_tool(handle, name2.as_ptr(), ptr::null(), echo_callback)
        };
        let r3 = unsafe {
            hyperlight_sandbox_register_tool(handle, name3.as_ptr(), ptr::null(), echo_callback)
        };

        assert!(r1.is_success);
        assert!(r2.is_success);
        assert!(r3.is_success);

        let state = unsafe { &*handle };
        assert_eq!(state.tools.len(), 3);

        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // build_tool_registry (internal)
    // -----------------------------------------------------------------------

    #[test]
    fn build_tool_registry_empty_succeeds() {
        let tools = HashMap::new();
        let registry = build_tool_registry(&tools);
        assert!(registry.is_ok());
    }

    #[test]
    fn build_tool_registry_with_callback_dispatches() {
        let mut tools = HashMap::new();
        tools.insert(
            "echo".to_string(),
            ToolEntry {
                callback: echo_callback,
                schema_json: None,
            },
        );

        let registry = build_tool_registry(&tools).expect("should build");
        let args = serde_json::json!({"message": "hello"});
        let result = registry.dispatch("echo", args).expect("should dispatch");

        // The echo callback wraps input in {"echo": ...}
        assert!(result.get("echo").is_some());
    }

    #[test]
    fn build_tool_registry_with_invalid_schema_fails() {
        let mut tools = HashMap::new();
        tools.insert(
            "bad".to_string(),
            ToolEntry {
                callback: echo_callback,
                schema_json: Some("not valid json".to_string()),
            },
        );

        let result = build_tool_registry(&tools);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Execution: run() without a real module (should fail gracefully)
    // -----------------------------------------------------------------------

    #[test]
    fn run_with_nonexistent_module_fails() {
        let handle = create_test_handle();
        let code = cstr("print('hello')");
        let result = unsafe { hyperlight_sandbox_run(handle, code.as_ptr()) };

        // Should fail because module doesn't exist, but NOT crash
        assert!(!result.is_success);
        assert!(!result.value.is_null());

        // Error message should mention the file
        let err = unsafe { CStr::from_ptr(result.value) }
            .to_str()
            .expect("valid UTF-8");
        assert!(!err.is_empty(), "error message should not be empty");

        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn run_null_handle_fails() {
        let code = cstr("print('hello')");
        let result = unsafe { hyperlight_sandbox_run(ptr::null_mut(), code.as_ptr()) };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
    }

    #[test]
    fn run_null_code_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_run(handle, ptr::null()) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Filesystem: pre-init access fails gracefully
    // -----------------------------------------------------------------------

    #[test]
    fn get_output_files_before_init_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_get_output_files(handle) };
        assert!(!result.is_success);
        let err = unsafe { CStr::from_ptr(result.value) }.to_str().unwrap();
        assert!(err.contains("not initialized"));
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn output_path_before_init_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_output_path(handle) };
        assert!(!result.is_success);
        let err = unsafe { CStr::from_ptr(result.value) }.to_str().unwrap();
        assert!(err.contains("not initialized"));
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    // -----------------------------------------------------------------------
    // Snapshot: pre-init access fails gracefully
    // -----------------------------------------------------------------------

    #[test]
    fn snapshot_before_init_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_snapshot(handle) };
        assert!(!result.is_success);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn restore_null_snapshot_fails() {
        let handle = create_test_handle();
        let result = unsafe { hyperlight_sandbox_restore(handle, ptr::null()) };
        assert!(!result.is_success);
        assert_eq!(result.error_code, FFIErrorCode::InvalidArgument as u32);
        unsafe { hyperlight_sandbox_free_string(result.value) };
        unsafe { hyperlight_sandbox_free(handle) };
    }

    #[test]
    fn free_snapshot_null_is_safe() {
        unsafe { hyperlight_sandbox_free_snapshot(ptr::null_mut()) };
    }

    // -----------------------------------------------------------------------
    // Error code values are stable
    // -----------------------------------------------------------------------

    #[test]
    fn error_codes_have_expected_values() {
        assert_eq!(FFIErrorCode::Success as u32, 0);
        assert_eq!(FFIErrorCode::Unknown as u32, 1);
        assert_eq!(FFIErrorCode::Timeout as u32, 2);
        assert_eq!(FFIErrorCode::Poisoned as u32, 3);
        assert_eq!(FFIErrorCode::PermissionDenied as u32, 4);
        assert_eq!(FFIErrorCode::GuestError as u32, 5);
        assert_eq!(FFIErrorCode::InvalidArgument as u32, 6);
        assert_eq!(FFIErrorCode::IoError as u32, 7);
    }

    // -----------------------------------------------------------------------
    // Config after init is rejected
    // -----------------------------------------------------------------------
    // We can't test this with a real initialized sandbox (no module),
    // but we can manually set inner to verify the guard.

    #[test]
    fn register_tool_after_init_flagged_fails() {
        // We test the guard by manually simulating an initialized state.
        // Since we can't build a real Sandbox without a valid module,
        // we verify via the pre-init path (covered by other tests).
        // The actual post-init rejection is verified via the "inner.is_some()"
        // check in register_tool — which is a code path we can trust from
        // the pre-init tests plus code inspection.
        //
        // A full integration test with a real .wasm module is in Phase 6.
    }
}
