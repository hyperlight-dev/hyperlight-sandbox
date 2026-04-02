//! Network access demo — tests network allow/deny for the JavaScript Wasm sandbox.

use std::path::Path;

use hyperlight_sandbox::{DEFAULT_HEAP_SIZE, DEFAULT_STACK_SIZE, Sandbox};
use hyperlight_wasm_sandbox::Wasm;

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

fn main() {
    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(javascript_guest_path())
        .heap_size(DEFAULT_HEAP_SIZE)
        .stack_size(DEFAULT_STACK_SIZE)
        .build()
        .expect("failed to create JS sandbox");
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
try {
    const resp = await fetch('https://notallowed.example');
    console.log('Got response: ' + resp.status);
} catch (e) {
    console.log('Network blocked: ' + e.message);
    console.log('  (notallowed.example is not in the allowlist — correct!)');
}
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
const resp = await fetch('https://httpbin.org/get');
const body = await resp.text();
console.log('HTTP status: ' + resp.status);
console.log('Response body (first 200 chars):');
console.log(body.slice(0, 200));
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
try {
    const resp = await fetch('https://httpbin.org/get');
    console.log('GET allowed: status ' + resp.status);
} catch (e) {
    console.log('GET result: ' + e.message);
}
try {
    const resp = await fetch('https://httpbin.org/post', { method: 'POST' });
    console.log('POST allowed: status ' + resp.status);
} catch (e) {
    console.log('POST blocked: ' + e.message);
    console.log('  (httpbin.org only allows GET — correct!)');
}
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
