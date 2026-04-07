//! Basics demo for the JavaScript Wasm component guest.
//!
//! Exercises: basic execution, tool dispatch, snapshot/restore, computation,
//! and nested tool calls.
//!
//! File I/O tests live in js_filesystem_demo.rs.
//! Network tests live in js_network_demo.rs.

use std::path::Path;

use hyperlight_sandbox::{SandboxBuilder, ToolRegistry};
use hyperlight_wasm_sandbox::Wasm;
use serde::Deserialize;

fn javascript_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/javascript/js-sandbox.aot")
        .display()
        .to_string()
}

fn separator(title: &str) {
    println!("\n{}", "═".repeat(60));
    println!("{title}");
    println!("{}", "═".repeat(60));
}

#[derive(Deserialize)]
struct MathArgs {
    a: f64,
    b: f64,
}

#[derive(Deserialize)]
struct GreetArgs {
    #[serde(default = "default_name")]
    name: String,
}

fn default_name() -> String {
    "world".to_string()
}

#[derive(Deserialize)]
struct LookupArgs {
    #[serde(default)]
    key: String,
}

fn main() {
    let mut tools = ToolRegistry::new();
    tools.register_typed::<MathArgs, _>("add", |args| Ok(serde_json::json!(args.a + args.b)));
    tools.register_typed::<MathArgs, _>("multiply", |args| Ok(serde_json::json!(args.a * args.b)));
    tools.register_typed::<GreetArgs, _>("greet", |args| {
        Ok(serde_json::json!(format!("Hello, {}!", args.name)))
    });
    tools.register_typed::<LookupArgs, _>("lookup", |args| {
        let val = match args.key.as_str() {
            "api_key" => "sk-demo",
            "model" => "gpt-4",
            _ => "not found",
        };
        Ok(serde_json::json!(val))
    });

    let mut sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(javascript_guest_path())
        .with_tools(tools)
        .build()
        .expect("failed to create JS sandbox");

    // ── Test 1: Basic code execution ────────────────────────────────
    separator("Test 1: Basic code execution");
    let result = sandbox
        .run(
            r#"
const primes = [];
for (let n = 2; n < 50; n++) {
    let isPrime = true;
    for (let i = 2; i <= Math.sqrt(n); i++) {
        if (n % i === 0) { isPrime = false; break; }
    }
    if (isPrime) primes.push(n);
}
console.log('Primes under 50: [' + primes.join(', ') + ']');
console.log('Count: ' + primes.length);
"#,
        )
        .expect("test 1 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("Count: 15"),
        "stdout: {:?}",
        result.stdout
    );

    // ── Test 2: Tool dispatch — host functions from guest code ──────
    separator("Test 2: Tool dispatch — host functions from guest code");
    let result = sandbox
        .run(
            r#"
const sum = call_tool('add', { a: 10, b: 20 });
const product = call_tool('multiply', { a: 6, b: 7 });
const greeting = call_tool('greet', { name: 'Developer' });
const config = call_tool('lookup', { key: 'model' });

console.log('10 + 20 = ' + sum);
console.log('6 x 7 = ' + product);
console.log(greeting);
console.log('Config lookup: model = ' + config);

try {
    call_tool('nonexistent_tool');
} catch (e) {
    console.log('Error handling works: ' + e.message);
}
"#,
        )
        .expect("test 2 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("10 + 20 = 30"),
        "stdout: {:?}",
        result.stdout
    );
    assert!(
        result.stdout.contains("6 x 7 = 42"),
        "stdout: {:?}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Hello, Developer!"),
        "stdout: {:?}",
        result.stdout
    );

    // ── Test 3: Snapshot/restore — rewind interpreter state ─────────
    separator("Test 3: Snapshot/restore — rewind interpreter state");
    let snap = sandbox.snapshot().expect("snapshot failed");

    let result = sandbox
        .run("globalThis.counter = 100; console.log('Set counter = ' + globalThis.counter);")
        .expect("run failed");
    println!("Before restore: {}", result.stdout.trim());

    sandbox.restore(&snap).expect("restore failed");

    let result = sandbox
        .run(
            r#"
if (globalThis.counter !== undefined) {
    console.log('counter = ' + globalThis.counter);
} else {
    console.log('counter is undefined — state was rolled back!');
}
"#,
        )
        .expect("run failed");
    println!("After restore:  {}", result.stdout.trim());
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("state was rolled back"),
        "stdout: {:?}",
        result.stdout
    );

    // ── Test 4: Complex multi-step computation ──────────────────────
    separator("Test 4: Complex multi-step computation");
    let result = sandbox
        .run(
            r#"
const data = [];
for (let i = 0; i < 5; i++) {
    data.push(call_tool('multiply', { a: i, b: i }));
}
const firstThree = data.slice(0, 3).reduce((a, b) => a + b, 0);
const lastTwo = data.slice(3).reduce((a, b) => a + b, 0);
const total = call_tool('add', { a: firstThree, b: lastTwo });
console.log('Squares: [' + data.join(', ') + ']');
console.log('Total: ' + total);
"#,
        )
        .expect("test 4 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);
    assert!(
        result.stdout.contains("Total: 30"),
        "stdout: {:?}",
        result.stdout
    );

    // ── Test 5: Nested tool calls ───────────────────────────────────
    separator("Test 5: Nested tool calls");
    let result = sandbox
        .run(
            r#"
// (3 + 4) * 5 = 35
const nested = call_tool('multiply', { a: call_tool('add', { a: 3, b: 4 }), b: 5 });
console.log('(3 + 4) * 5 = ' + nested);

// (2 * 3) + (4 * 5) = 26
const deep = call_tool('add', {
    a: call_tool('multiply', { a: 2, b: 3 }),
    b: call_tool('multiply', { a: 4, b: 5 }),
});
console.log('(2 * 3) + (4 * 5) = ' + deep);

const greeting = call_tool('greet', { name: call_tool('lookup', { key: 'model' }) });
console.log('Greeting with lookup: ' + greeting);
"#,
        )
        .expect("test 5 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("35"), "stdout: {:?}", result.stdout);
    assert!(result.stdout.contains("26"), "stdout: {:?}", result.stdout);
    assert!(
        result.stdout.contains("gpt-4"),
        "stdout: {:?}",
        result.stdout
    );

    separator("All tests passed!");
}
