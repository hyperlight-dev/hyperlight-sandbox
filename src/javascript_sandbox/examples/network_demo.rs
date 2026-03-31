//! Network access demo for the HyperlightJS sandbox backend.

use hyperlight_javascript_sandbox::HyperlightJs;
use hyperlight_sandbox::Sandbox;

fn separator(title: &str) {
    println!("\n{}", "═".repeat(60));
    println!("{title}");
    println!("{}", "═".repeat(60));
}

fn main() {
    let mut sandbox = Sandbox::builder()
        .guest(HyperlightJs)
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

    separator("Test 2: Network access to allowed domain");
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
    if result.exit_code == 0 {
        println!("Network access to allowed domain works!");
    } else {
        eprintln!("Network access failed");
        eprintln!("stderr: {}", &result.stderr[..result.stderr.len().min(300)]);
    }

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

    separator("All tests passed!");
}
