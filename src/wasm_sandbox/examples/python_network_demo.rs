//! Network access demo — tests network allow/deny for the Python Wasm sandbox.

use std::path::Path;

use hyperlight_sandbox::Sandbox;
use hyperlight_wasm_sandbox::Wasm;

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

fn main() {
    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .heap_size(200 * 1024 * 1024)
        .stack_size(100 * 1024 * 1024)
        .build()
        .expect("failed to create sandbox");
    sandbox
        .allow_domain(
            "https://httpbin.org",
            vec![hyperlight_sandbox::HttpMethod::Get],
        )
        .unwrap();

    separator("Test 1: Network access denied without permissions");
    let result = sandbox
        .run(
            r#"
try:
    resp = http_get("https://notallowed.example")
    print(f"Got response: {resp['status']}")
except Exception as e:
    print(f"Network blocked: {type(e).__name__}: {e}")
    print("  (notallowed.example is not in the allowlist — correct!)")
"#,
        )
        .expect("test 1 failed");
    print!("{}", result.stdout);
    assert!(
        result.stdout.contains("Network blocked"),
        "test 1: expected network access to be blocked"
    );

    separator("Test 2: Network access to allowed domain (WASI-HTTP)");
    let result = sandbox
        .run(
            r#"
resp = http_get("https://httpbin.org/get")
print(f"HTTP status: {resp['status']}")
print(f"Response body (first 200 chars):")
print(resp['body'][:200])
"#,
        )
        .expect("test 2 failed");
    print!("{}", result.stdout);
    assert_eq!(
        result.exit_code,
        0,
        "test 2: network access to allowed domain failed\nstderr: {}",
        &result.stderr[..result.stderr.len().min(300)]
    );

    separator("Test 3: Method filtering — GET allowed, POST blocked");
    let result = sandbox
        .run(
            r#"
try:
    resp = http_get("https://httpbin.org/get")
    print(f"GET allowed: status {resp['status']}")
except Exception as e:
    print(f"GET failed: {e}")

try:
    resp = http_post("https://httpbin.org/post", body='{"test": 1}')
    print(f"POST allowed: status {resp['status']}")
except Exception as e:
    print(f"POST blocked: {e}")
    print("  (httpbin.org only allows GET — correct!)")
"#,
        )
        .expect("test 3 failed");
    print!("{}", result.stdout);
    assert!(
        result.stdout.contains("POST blocked"),
        "test 3: expected POST to be blocked"
    );

    separator("All tests passed!");
}
