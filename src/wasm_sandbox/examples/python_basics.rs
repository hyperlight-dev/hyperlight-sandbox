//! Basics demo — execution, tools, snapshot/restore, computation, nested tools.
//!
//! File I/O tests live in python_filesystem_demo.rs.
//! Network tests live in python_network_demo.rs.

use std::path::Path;

use hyperlight_sandbox::{DEFAULT_HEAP_SIZE, DEFAULT_STACK_SIZE, Sandbox, ToolRegistry};
use hyperlight_wasm_sandbox::Wasm;
use serde::Deserialize;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
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

    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .heap_size(DEFAULT_HEAP_SIZE)
        .stack_size(DEFAULT_STACK_SIZE)
        .with_tools(tools)
        .build()
        .expect("failed to create sandbox");

    separator("Test 1: Basic code execution");
    let result = sandbox
        .run(
            r#"
import math
primes = [n for n in range(2, 50) if all(n % i != 0 for i in range(2, int(math.sqrt(n)) + 1))]
print(f"Primes under 50: {primes}")
print(f"Count: {len(primes)}")
"#,
        )
        .expect("test 1 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);

    separator("Test 2: Tool dispatch — host functions from guest code");
    let result = sandbox
        .run(
            r#"
sum_result = call_tool('add', a=10, b=20)
product = call_tool('multiply', a=6, b=7)
greeting = call_tool('greet', name='Developer')
config = call_tool('lookup', key='model')

print(f"10 + 20 = {sum_result}")
print(f"6 x 7 = {product}")
print(f"{greeting}")
print(f"Config lookup: model = {config}")

try:
    call_tool('nonexistent_tool')
except RuntimeError as e:
    print(f"Error handling works: {e}")
"#,
        )
        .expect("test 2 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);

    separator("Test 3: Snapshot/restore — rewind interpreter state");
    let snap = sandbox.snapshot().expect("snapshot failed");

    let result = sandbox
        .run("counter = 100; print(f'Set counter = {counter}')")
        .expect("run failed");
    println!("Before restore: {}", result.stdout.trim());

    sandbox.restore(&snap).expect("restore failed");

    let result = sandbox
        .run(
            r#"
try:
    print(f"counter = {counter}")
except NameError:
    print("counter is undefined — state was rolled back!")
"#,
        )
        .expect("run failed");
    println!("After restore:  {}", result.stdout.trim());
    assert_eq!(result.exit_code, 0);

    separator("Test 4: Complex multi-step computation");
    let result = sandbox
        .run(
            r#"
data = []
for i in range(5):
    val = call_tool('multiply', a=i, b=i)
    data.append(val)
total = call_tool('add', a=sum(data[:3]), b=sum(data[3:]))
print(f"Squares: {data}")
print(f"Total: {total}")
"#,
        )
        .expect("test 4 failed");
    print!("{}", result.stdout);
    assert_eq!(result.exit_code, 0);

    separator("Test 5: Nested tool calls");
    let result = sandbox
        .run(
            r#"
# (3 + 4) * 5 = 35
nested = call_tool('multiply', a=call_tool('add', a=3, b=4), b=5)
print(f"(3 + 4) * 5 = {nested}")

# (2 * 3) + (4 * 5) = 26
deep = call_tool('add',
    a=call_tool('multiply', a=2, b=3),
    b=call_tool('multiply', a=4, b=5),
)
print(f"(2 * 3) + (4 * 5) = {deep}")

greeting = call_tool('greet', name=call_tool('lookup', key='model'))
print(f"Greeting with lookup: {greeting}")
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
