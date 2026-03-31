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
const resp = await fetch('https://notallowed.example');
if (resp.status === 403) {
    console.log('Network blocked: status ' + resp.status);
    console.log('  (notallowed.example is not in the allowlist — correct!)');
} else {
    console.log('Got response: ' + resp.status);
}
"#,
        )
        .expect("test 1 failed");
    print!("{}", result.stdout);
    assert!(
        result.stdout.contains("Network blocked"),
        "test 1: expected network access to be blocked"
    );

    separator("Test 2: Network access to allowed domain");
    let result = sandbox
        .run(
            r#"
const resp = await fetch('https://httpbin.org/json', { headers: { 'accept': 'application/json' } });
const body = await resp.text();
console.log('HTTP status: ' + resp.status);
console.log(body);
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
const getResp = await fetch('https://httpbin.org/get');
if (getResp.status === 200) {
    const body = await getResp.text();
    console.log('GET allowed: status ' + getResp.status);
    console.log(body);
} else {
    console.log('GET failed: status ' + getResp.status);
}
const postResp = await fetch('https://httpbin.org/post', { method: 'POST' });
if (postResp.status === 403) {
    console.log('POST blocked: status ' + postResp.status);
    console.log('  (httpbin.org only allows GET — correct!)');
} else {
    console.log('POST allowed: status ' + postResp.status);
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
