//! Tool schema, registry, request parsing, and argument validation.

use std::collections::HashMap;

use anyhow::Result;

/// Expected JSON type for a tool argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgType {
    Number,
    String,
    Boolean,
    Object,
    Array,
}

impl ArgType {
    fn matches(&self, value: &serde_json::Value) -> bool {
        match self {
            Self::Number => value.is_number(),
            Self::String => value.is_string(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }
}

/// Schema describing expected arguments for a tool.
#[derive(Debug, Clone, Default)]
pub struct ToolSchema {
    /// Maps argument name → expected type (only for args with known types).
    pub properties: HashMap<std::string::String, ArgType>,
    /// Arguments that must be present (checked regardless of type).
    pub required: Vec<std::string::String>,
}

impl ToolSchema {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a required argument with a known type.
    pub fn required_arg(mut self, name: &str, arg_type: ArgType) -> Self {
        self.required.push(name.to_string());
        self.properties.insert(name.to_string(), arg_type);
        self
    }

    /// Add a required argument whose type is unknown.
    /// Presence is enforced but no type check is performed.
    pub fn required_untyped(mut self, name: &str) -> Self {
        self.required.push(name.to_string());
        self
    }

    /// Add an optional argument with the given type.
    pub fn optional_arg(mut self, name: &str, arg_type: ArgType) -> Self {
        self.properties.insert(name.to_string(), arg_type);
        self
    }

    /// Validate arguments against this schema.
    fn validate(&self, tool_name: &str, args: &serde_json::Value) -> Result<()> {
        let obj = args.as_object().ok_or_else(|| {
            anyhow::anyhow!("tool '{tool_name}': arguments must be a JSON object")
        })?;

        // Check required arguments are present.
        for req in &self.required {
            if !obj.contains_key(req) {
                anyhow::bail!("tool '{tool_name}': missing required argument '{req}'");
            }
        }

        // Check types of provided arguments.
        for (key, expected_type) in &self.properties {
            if let Some(value) = obj.get(key)
                && !expected_type.matches(value)
            {
                anyhow::bail!(
                    "tool '{tool_name}': argument '{key}' must be {}, got {}",
                    expected_type.type_name(),
                    json_type_name(value),
                );
            }
        }

        Ok(())
    }
}

/// Internal entry pairing a handler with an optional schema.
struct ToolEntry {
    handler: Box<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>,
    schema: Option<ToolSchema>,
}

/// Registry of host-side tool handlers.
///
/// Thread-safe for concurrent `dispatch()` calls after initialization.
/// Do not mutate (register tools) after passing to a `SandboxBuilder`.
pub struct ToolRegistry {
    tools: HashMap<std::string::String, ToolEntry>,
    /// Maximum size in bytes for the serialized arguments JSON.
    /// Applied in `dispatch()` before any parsing or handler invocation.
    /// Set to 0 to disable the limit.
    max_args_size: usize,
}

/// Default maximum argument payload size (1 MiB).
const DEFAULT_MAX_ARGS_SIZE: usize = 1024 * 1024;

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            max_args_size: DEFAULT_MAX_ARGS_SIZE,
        }
    }

    /// Set the maximum allowed size (in bytes) for serialized tool arguments.
    /// Set to 0 to disable the limit.
    pub fn set_max_args_size(&mut self, max_bytes: usize) {
        self.max_args_size = max_bytes;
    }

    /// Register a tool handler that explicitly opts out of argument validation.
    /// Use this only for tools that handle raw JSON and validate internally.
    pub fn register_unvalidated<F>(&mut self, name: &str, handler: F)
    where
        F: Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        self.register_with_schema(name, None, handler);
    }

    /// Register a tool handler with an optional schema for argument validation.
    /// Pass `Some(schema)` to validate arguments, or `None` to skip validation.
    pub fn register_with_schema<F>(&mut self, name: &str, schema: Option<ToolSchema>, handler: F)
    where
        F: Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        self.tools.insert(
            name.to_string(),
            ToolEntry {
                handler: Box::new(handler),
                schema,
            },
        );
    }

    /// Register a typed tool handler that automatically deserializes arguments
    /// and rejects mismatched types via serde.
    pub fn register_typed<T, F>(&mut self, name: &str, handler: F)
    where
        T: serde::de::DeserializeOwned + Send + 'static,
        F: Fn(T) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        let tool_name = name.to_string();
        self.tools.insert(
            name.to_string(),
            ToolEntry {
                handler: Box::new(move |args| {
                    let typed: T = serde_json::from_value(args).map_err(|e| {
                        anyhow::anyhow!("tool '{tool_name}': invalid arguments: {e}")
                    })?;
                    handler(typed)
                }),
                schema: None,
            },
        );
    }

    /// Dispatch a tool call — checks payload size, validates against schema
    /// (if present), then invokes the handler.
    pub fn dispatch(&self, name: &str, args: serde_json::Value) -> Result<serde_json::Value> {
        // Payload size check — runs before everything else to prevent DoS.
        // Uses a cheap recursive estimator instead of serializing to string.
        if self.max_args_size > 0 {
            let size = estimate_json_size(&args);
            if size > self.max_args_size {
                anyhow::bail!(
                    "tool '{name}': argument payload too large ({size} bytes, max {})",
                    self.max_args_size
                );
            }
        }

        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", name))?;

        // Schema validation runs before handler.
        if let Some(schema) = &tool.schema {
            schema.validate(name, &args)?;
        }

        (tool.handler)(args)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Return a human-readable type name for a JSON value.
fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Maximum nesting depth for `estimate_json_size` to prevent stack overflow.
const MAX_JSON_DEPTH: usize = 64;

/// Estimate the serialized JSON size of a value without allocating a string.
/// Conservative upper bound — may slightly overcount due to escape sequences
/// but never undercounts.
fn estimate_json_size(v: &serde_json::Value) -> usize {
    estimate_json_size_inner(v, 0)
}

fn estimate_json_size_inner(v: &serde_json::Value, depth: usize) -> usize {
    if depth > MAX_JSON_DEPTH {
        return 0;
    }
    match v {
        serde_json::Value::Null => 4, // "null"
        serde_json::Value::Bool(b) => {
            if *b {
                4
            } else {
                5
            }
        } // "true" / "false"
        serde_json::Value::Number(n) => n.to_string().len(),
        serde_json::Value::String(s) => s.len() + 2,
        serde_json::Value::Array(arr) => {
            let inner: usize = arr
                .iter()
                .map(|v| estimate_json_size_inner(v, depth + 1))
                .sum();
            2 + inner + arr.len().saturating_sub(1)
        }
        serde_json::Value::Object(map) => {
            let inner: usize = map
                .iter()
                .map(|(k, v)| k.len() + 2 + 1 + estimate_json_size_inner(v, depth + 1))
                .sum();
            2 + inner + map.len().saturating_sub(1)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_tool_returns_error() {
        let registry = ToolRegistry::new();
        let err = registry
            .dispatch("nonexistent", serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }

    #[test]
    fn register_and_dispatch_simple_tool() {
        let mut registry = ToolRegistry::new();
        registry.register_unvalidated("echo", Ok);
        let result = registry
            .dispatch("echo", serde_json::json!({"msg": "hi"}))
            .unwrap();
        assert_eq!(result, serde_json::json!({"msg": "hi"}));
    }

    #[derive(serde::Deserialize)]
    struct AddArgs {
        a: f64,
        b: f64,
    }

    #[test]
    fn register_typed_accepts_correct_types() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let result = registry
            .dispatch("add", serde_json::json!({"a": 3.0, "b": 4.0}))
            .unwrap();
        assert_eq!(result, serde_json::json!(7.0));
    }

    #[test]
    fn register_typed_rejects_string_instead_of_number() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": "three", "b": 4.0}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid arguments"));
    }

    #[test]
    fn register_typed_rejects_missing_field() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": 3.0}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid arguments"));
    }

    #[test]
    fn register_typed_rejects_boolean_instead_of_number() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": true, "b": 4.0}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid arguments"));
    }

    #[test]
    fn register_typed_rejects_array_instead_of_number() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": [1, 2], "b": 4.0}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid arguments"));
    }

    #[test]
    fn register_typed_rejects_null_instead_of_number() {
        let mut registry = ToolRegistry::new();
        registry.register_typed::<AddArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": null, "b": 4.0}))
            .unwrap_err();
        assert!(err.to_string().contains("invalid arguments"));
    }

    #[test]
    fn schema_rejects_string_where_number_expected() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new()
            .required_arg("a", ArgType::Number)
            .required_arg("b", ArgType::Number);
        registry.register_with_schema("add", Some(schema), |args| {
            Ok(serde_json::json!(
                args["a"].as_f64().unwrap() + args["b"].as_f64().unwrap()
            ))
        });
        let err = registry
            .dispatch("add", serde_json::json!({"a": "three", "b": 4}))
            .unwrap_err();
        assert!(err.to_string().contains("must be number"));
        assert!(err.to_string().contains("got string"));
    }

    #[test]
    fn schema_rejects_number_where_string_expected() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new().required_arg("name", ArgType::String);
        registry.register_with_schema("greet", Some(schema), |args| {
            Ok(serde_json::json!(format!(
                "Hello, {}!",
                args["name"].as_str().unwrap()
            )))
        });
        let err = registry
            .dispatch("greet", serde_json::json!({"name": 42}))
            .unwrap_err();
        assert!(err.to_string().contains("must be string"));
        assert!(err.to_string().contains("got number"));
    }

    #[test]
    fn schema_rejects_missing_required_arg() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new()
            .required_arg("a", ArgType::Number)
            .required_arg("b", ArgType::Number);
        registry.register_with_schema("add", Some(schema), |_| Ok(serde_json::json!(0)));
        let err = registry
            .dispatch("add", serde_json::json!({"a": 1}))
            .unwrap_err();
        assert!(err.to_string().contains("missing required argument 'b'"));
    }

    #[test]
    fn schema_accepts_correct_types() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new()
            .required_arg("a", ArgType::Number)
            .required_arg("b", ArgType::Number);
        registry.register_with_schema("add", Some(schema), |args| {
            Ok(serde_json::json!(
                args["a"].as_f64().unwrap() + args["b"].as_f64().unwrap()
            ))
        });
        let result = registry
            .dispatch("add", serde_json::json!({"a": 3, "b": 4}))
            .unwrap();
        assert_eq!(result, serde_json::json!(7.0));
    }

    #[test]
    fn schema_optional_arg_allows_absence() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new()
            .required_arg("a", ArgType::Number)
            .optional_arg("b", ArgType::Number);
        registry.register_with_schema("add", Some(schema), |args| {
            let a = args["a"].as_f64().unwrap();
            let b = args["b"].as_f64().unwrap_or(0.0);
            Ok(serde_json::json!(a + b))
        });
        let result = registry
            .dispatch("add", serde_json::json!({"a": 5}))
            .unwrap();
        assert_eq!(result, serde_json::json!(5.0));
    }

    #[test]
    fn schema_optional_arg_validates_type_when_present() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new().optional_arg("x", ArgType::Number);
        registry.register_with_schema("f", Some(schema), |_| Ok(serde_json::json!(0)));
        let err = registry
            .dispatch("f", serde_json::json!({"x": "bad"}))
            .unwrap_err();
        assert!(err.to_string().contains("must be number"));
    }

    #[test]
    fn schema_required_untyped_checks_presence_not_type() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new().required_untyped("x");
        registry.register_with_schema("f", Some(schema), |_| Ok(serde_json::json!("ok")));

        assert!(
            registry
                .dispatch("f", serde_json::json!({"x": "string"}))
                .is_ok()
        );
        assert!(registry.dispatch("f", serde_json::json!({"x": 42})).is_ok());
        assert!(
            registry
                .dispatch("f", serde_json::json!({"x": [1, 2]}))
                .is_ok()
        );

        let err = registry.dispatch("f", serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("missing required argument 'x'"));
    }

    #[test]
    fn schema_rejects_non_object_args() {
        let mut registry = ToolRegistry::new();
        let schema = ToolSchema::new().required_arg("a", ArgType::Number);
        registry.register_with_schema("f", Some(schema), |_| Ok(serde_json::json!(0)));
        let err = registry
            .dispatch("f", serde_json::json!("not an object"))
            .unwrap_err();
        assert!(err.to_string().contains("must be a JSON object"));
    }

    #[test]
    fn argtype_matches_correct_json_types() {
        assert!(ArgType::Number.matches(&serde_json::json!(42)));
        assert!(ArgType::Number.matches(&serde_json::json!(3.15)));
        assert!(ArgType::String.matches(&serde_json::json!("hello")));
        assert!(ArgType::Boolean.matches(&serde_json::json!(true)));
        assert!(ArgType::Object.matches(&serde_json::json!({"k": "v"})));
        assert!(ArgType::Array.matches(&serde_json::json!([1, 2, 3])));
    }

    #[test]
    fn argtype_rejects_wrong_json_types() {
        assert!(!ArgType::Number.matches(&serde_json::json!("42")));
        assert!(!ArgType::Number.matches(&serde_json::json!(true)));
        assert!(!ArgType::Number.matches(&serde_json::json!(null)));
        assert!(!ArgType::String.matches(&serde_json::json!(42)));
        assert!(!ArgType::Boolean.matches(&serde_json::json!(1)));
        assert!(!ArgType::Object.matches(&serde_json::json!([1])));
        assert!(!ArgType::Array.matches(&serde_json::json!({"k": "v"})));
    }

    #[test]
    fn dispatch_rejects_oversized_payload() {
        let mut registry = ToolRegistry::new();
        registry.set_max_args_size(100);
        registry.register_unvalidated("echo", Ok);
        let big = "x".repeat(200);
        let err = registry
            .dispatch("echo", serde_json::json!({"data": big}))
            .unwrap_err();
        assert!(err.to_string().contains("payload too large"));
    }

    #[test]
    fn dispatch_accepts_payload_within_limit() {
        let mut registry = ToolRegistry::new();
        registry.set_max_args_size(1000);
        registry.register_unvalidated("echo", Ok);
        let small = "x".repeat(10);
        assert!(
            registry
                .dispatch("echo", serde_json::json!({"data": small}))
                .is_ok()
        );
    }

    #[test]
    fn dispatch_with_zero_limit_disables_check() {
        let mut registry = ToolRegistry::new();
        registry.set_max_args_size(0);
        registry.register_unvalidated("echo", Ok);
        let big = "x".repeat(10_000_000);
        assert!(
            registry
                .dispatch("echo", serde_json::json!({"data": big}))
                .is_ok()
        );
    }

    #[test]
    fn default_max_args_size_is_1mib() {
        let registry = ToolRegistry::new();
        assert_eq!(registry.max_args_size, 1024 * 1024);
    }

    #[test]
    fn estimate_json_size_is_within_reasonable_bounds() {
        let test_cases: Vec<serde_json::Value> = vec![
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(42),
            serde_json::json!(3.15),
            serde_json::json!(-999999),
            serde_json::json!("hello world"),
            serde_json::json!(""),
            serde_json::json!({"a": 1, "b": "two", "c": true}),
            serde_json::json!([1, "two", null, false]),
            serde_json::json!({"nested": {"deep": {"value": 42}}}),
            serde_json::json!({"list": [1, 2, 3], "name": "test"}),
            serde_json::json!({"x": "x".repeat(1000)}),
        ];

        for val in &test_cases {
            let estimated = estimate_json_size(val);
            let actual = val.to_string().len();
            let lower = actual * 95 / 100;
            let upper = actual * 105 / 100;
            assert!(
                estimated >= lower,
                "estimate {estimated} too low (actual {actual}, min {lower}): {val}"
            );
            assert!(
                estimated <= upper,
                "estimate {estimated} too high (actual {actual}, max {upper}): {val}"
            );
        }
    }

    #[test]
    fn code_in_args_is_treated_as_inert_data() {
        let mut registry = ToolRegistry::new();
        registry.register_unvalidated("echo", Ok);

        // Payloads that would be dangerous if eval'd but are harmless as data.
        let payloads = vec![
            serde_json::json!({"cmd": "__import__('sys').exit(1)"}),
            serde_json::json!({"nested": {"__proto__": {"polluted": true}}}),
            serde_json::json!({"constructor": {"prototype": {"isAdmin": true}}}),
        ];

        for payload in &payloads {
            let result = registry.dispatch("echo", payload.clone()).unwrap();
            assert_eq!(
                result, *payload,
                "tool args must pass through as inert data"
            );
        }
    }

    #[test]
    fn json_edge_cases_do_not_crash_dispatch() {
        let mut registry = ToolRegistry::new();
        registry.set_max_args_size(0); // disable limit for these tests
        registry.register_unvalidated("echo", Ok);

        // Null bytes in strings
        let null_byte = serde_json::json!({"data": "before\0after"});
        let result = registry.dispatch("echo", null_byte.clone()).unwrap();
        assert_eq!(result, null_byte, "null bytes must survive round-trip");

        // Deeply nested objects (potential stack overflow in recursive parsers)
        let mut nested = serde_json::json!("leaf");
        for _ in 0..100 {
            nested = serde_json::json!({"d": nested});
        }
        let result = registry
            .dispatch("echo", serde_json::json!({"data": nested.clone()}))
            .unwrap();
        assert_eq!(result["data"], nested);

        // Huge number values
        let huge = serde_json::json!({"n": u64::MAX, "neg": i64::MIN});
        let result = registry.dispatch("echo", huge.clone()).unwrap();
        assert_eq!(result, huge);

        // Unicode edge cases: RTL override, zero-width joiner, homoglyphs
        let unicode = serde_json::json!({
            "rtl": "admin\u{202E}txt.exe",
            "zwj": "a\u{200D}b",
            "bom": "\u{FEFF}data",
            "null_escape": "test\\u0000end",
        });
        let result = registry.dispatch("echo", unicode.clone()).unwrap();
        assert_eq!(
            result, unicode,
            "unicode edge cases must survive round-trip"
        );

        // Empty and whitespace-heavy keys
        let odd_keys = serde_json::json!({"": "empty", " ": "space", "\t": "tab", "\n": "newline"});
        let result = registry.dispatch("echo", odd_keys.clone()).unwrap();
        assert_eq!(result, odd_keys);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    fn arb_json_leaf() -> impl Strategy<Value = serde_json::Value> {
        prop_oneof![
            Just(serde_json::Value::Null),
            any::<bool>().prop_map(serde_json::Value::Bool),
            any::<f64>()
                .prop_filter("finite", |f| f.is_finite())
                .prop_map(|f| serde_json::json!(f)),
            any::<i64>().prop_map(|i| serde_json::json!(i)),
            ".*".prop_map(|s: String| serde_json::Value::String(s)),
        ]
    }

    fn arb_json_value() -> impl Strategy<Value = serde_json::Value> {
        arb_json_leaf().prop_recursive(3, 32, 8, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..8).prop_map(serde_json::Value::Array),
                proptest::collection::hash_map("[a-z]{1,8}", inner, 0..8)
                    .prop_map(|map| { serde_json::Value::Object(map.into_iter().collect()) }),
            ]
        })
    }

    proptest! {
        #[test]
        fn typed_add_never_panics(a_val in arb_json_value(), b_val in arb_json_value()) {
            #[derive(serde::Deserialize)]
            struct AddArgs { a: f64, b: f64 }

            let mut registry = ToolRegistry::new();
            registry.set_max_args_size(0);
            registry.register_typed::<AddArgs, _>("add", |args| {
                Ok(serde_json::json!(args.a + args.b))
            });
            let args = serde_json::json!({"a": a_val, "b": b_val});
            let result = registry.dispatch("add", args);
            match result {
                Ok(v) => { let _ = v; }
                Err(e) => {
                    let msg = e.to_string();
                    prop_assert!(msg.contains("invalid arguments"), "unexpected error: {msg}");
                }
            }
        }

        #[test]
        fn schema_validation_never_panics(val in arb_json_value()) {
            let mut registry = ToolRegistry::new();
            registry.set_max_args_size(0);
            let schema = ToolSchema::new()
                .required_arg("x", ArgType::Number)
                .optional_arg("y", ArgType::String);
            registry.register_with_schema("f", Some(schema), |args| {
                let x = args["x"].as_f64().unwrap();
                let greeting = match args.get("y").and_then(|v| v.as_str()) {
                    Some(s) => format!("{s}: {x}"),
                    None => format!("{x}"),
                };
                Ok(serde_json::json!(greeting))
            });
            let args = serde_json::json!({"x": val});
            let _ = registry.dispatch("f", args);
        }

        #[test]
        fn payload_limit_rejects_large_strings(len in 1000usize..10000) {
            let mut registry = ToolRegistry::new();
            registry.set_max_args_size(500);
            registry.register_unvalidated("echo", Ok);
            let big = "x".repeat(len);
            let result = registry.dispatch("echo", serde_json::json!({"data": big}));
            prop_assert!(result.is_err());
            prop_assert!(result.unwrap_err().to_string().contains("payload too large"));
        }

        #[test]
        fn unvalidated_tool_with_real_handler_never_panics(val_a in arb_json_value(), val_b in arb_json_value()) {
            let mut registry = ToolRegistry::new();
            registry.set_max_args_size(0);
            registry.register_unvalidated("add", |args| {
                let a = args.get("a").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow::anyhow!("a must be a number"))?;
                let b = args.get("b").and_then(|v| v.as_f64())
                    .ok_or_else(|| anyhow::anyhow!("b must be a number"))?;
                Ok(serde_json::json!(a + b))
            });
            let args = serde_json::json!({"a": val_a, "b": val_b});
            let result = registry.dispatch("add", args);
            match (&val_a, &val_b) {
                (serde_json::Value::Number(_), serde_json::Value::Number(_)) => { let _ = result; }
                _ => {
                    prop_assert!(result.is_err(),
                        "expected Err for non-number args, got Ok: a={val_a}, b={val_b}");
                }
            }
        }

        #[test]
        fn arbitrary_tool_name_does_not_panic(name in ".*") {
            let registry = ToolRegistry::new();
            let result = registry.dispatch(&name, serde_json::json!({}));
            prop_assert!(result.is_err());
            prop_assert!(result.unwrap_err().to_string().contains("unknown tool"));
        }

        #[test]
        fn schema_mixed_args_consistent(
            a_val in arb_json_value(),
            b_val in arb_json_value(),
            name_val in arb_json_value(),
        ) {
            let mut registry = ToolRegistry::new();
            registry.set_max_args_size(0);
            let schema = ToolSchema::new()
                .required_arg("a", ArgType::Number)
                .required_arg("b", ArgType::Number)
                .optional_arg("name", ArgType::String);
            registry.register_with_schema("f", Some(schema), |args| {
                let a = args["a"].as_f64().unwrap();
                let b = args["b"].as_f64().unwrap();
                let label = args.get("name").and_then(|v| v.as_str()).unwrap_or("result");
                Ok(serde_json::json!({label: a + b}))
            });
            let args = serde_json::json!({"a": a_val, "b": b_val, "name": name_val});
            let result = registry.dispatch("f", args.clone());
            let a_ok = a_val.is_number();
            let b_ok = b_val.is_number();
            let name_ok = name_val.is_string();
            if a_ok && b_ok && name_ok {
                prop_assert!(result.is_ok(), "expected Ok, got {:?}", result);
            } else {
                prop_assert!(result.is_err(), "expected Err for args: {args}");
            }
        }
    }
}
