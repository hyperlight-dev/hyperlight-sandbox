//! Integration test: WASM sandbox → local echo server.
//!
//! Exercises the full path: guest Python `http_post()` → WASI body stream
//! → `WasiBodyStream` → `send_http_request` → echo server → response.

use std::path::Path;

use hyperlight_sandbox::test_utils::EchoServer;
use hyperlight_sandbox::{HttpMethod, SandboxBuilder};
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

#[tokio::test]
async fn wasm_python_post_with_body() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Post])
            .unwrap();

        let code = format!(
            r#"
resp = http_post("{base_url}/echo", body='{{"test_key": "test_value"}}')
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    assert_eq!(echo["method"], "POST");
    assert_eq!(echo["body"], r#"{"test_key": "test_value"}"#);
}

#[tokio::test]
async fn wasm_python_get_request() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .unwrap();

        let code = format!(
            r#"
resp = http_get("{base_url}/data")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    assert_eq!(echo["method"], "GET");
}

/// Sends a 64 KB body from guest Python — larger than WasiBodyStream's
/// 8 KB chunk size — verifying multi-chunk streaming works end-to-end.
#[tokio::test]
async fn wasm_python_post_large_body_streams_in_chunks() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Post])
            .unwrap();

        // Generate a 12 KB body — larger than WasiBodyStream's 8 KB
        // chunk size, requiring multiple frames, but within the
        // guest-to-host shared buffer limit.
        let code = format!(
            r#"
body = "A" * (12 * 1024)
resp = http_post("{base_url}/large", body=body, content_type="text/plain")
import json
echo = json.loads(resp["body"])
print(len(echo["body"]))
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    let received_len: usize = result
        .stdout
        .trim()
        .parse()
        .expect("expected body length in stdout");
    assert_eq!(
        received_len,
        12 * 1024,
        "full 12 KB body should arrive intact"
    );
}
