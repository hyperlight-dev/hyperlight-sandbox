//! Integration tests: scoped-credential injection end-to-end.
//!
//! Each test spins up a local [`EchoServer`], registers one or more
//! credentials, then runs guest Python code that exercises the
//! `credential=` kwarg on `http_get`/`http_post`.  The echo server
//! reflects all received headers so we can assert exactly what the
//! host injected (or blocked).

use std::path::Path;

use hyperlight_sandbox::test_utils::EchoServer;
use hyperlight_sandbox::{CredentialEntry, HttpMethod, SandboxBuilder};
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

/// Helper to build a [`CredentialEntry`] with sensible defaults.
fn cred(target: &str, resolver: &str) -> CredentialEntry {
    CredentialEntry {
        target: target.to_string(),
        header: "authorization".to_string(),
        prefix: "Bearer ".to_string(),
        resolver: resolver.to_string(),
    }
}

// -----------------------------------------------------------------------
// Happy path: credential header is injected
// -----------------------------------------------------------------------

#[tokio::test]
async fn credential_header_injected_on_get() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .register_credential("test_cred", cred(&base_url, "secret-token-42"))
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        let code = format!(
            r#"
resp = http_get("{base_url}/api/data", credential="test_cred")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    let headers = echo["headers"].as_object().expect("missing headers");
    assert_eq!(
        headers.get("authorization").and_then(|v| v.as_str()),
        Some("Bearer secret-token-42"),
        "credential header not injected or has wrong value"
    );
}

#[tokio::test]
async fn credential_header_injected_on_post() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .register_credential("post_cred", cred(&base_url, "post-token-99"))
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Post])
            .expect("allow_domain failed");

        let code = format!(
            r#"
resp = http_post("{base_url}/submit", body='{{"key": "val"}}', credential="post_cred")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    let headers = echo["headers"].as_object().expect("missing headers");
    assert_eq!(
        headers.get("authorization").and_then(|v| v.as_str()),
        Some("Bearer post-token-99"),
    );
}

// -----------------------------------------------------------------------
// Error: unknown credential name → guest RuntimeError
// -----------------------------------------------------------------------

#[tokio::test]
async fn unknown_credential_raises_error() {
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
            .expect("allow_domain failed");

        // Note: no credential registered — guest tries to attach "ghost"
        let code = format!(
            r#"
resp = http_get("{base_url}/api", credential="ghost")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_ne!(result.exit_code, 0, "expected non-zero exit code");
    assert!(
        result.stderr.contains("credential") || result.stderr.contains("RuntimeError"),
        "stderr should mention credential error, got: {}",
        result.stderr
    );
}

// -----------------------------------------------------------------------
// Error: scope mismatch — credential bound to different URL prefix
// -----------------------------------------------------------------------

#[tokio::test]
async fn scope_mismatch_denied() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        // Credential scoped to https://example.com — won't match the local server
        sandbox
            .register_credential("wrong_scope", cred("https://example.com/api", "nope"))
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        let code = format!(
            r#"
resp = http_get("{base_url}/api/data", credential="wrong_scope")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_ne!(
        result.exit_code, 0,
        "expected non-zero exit code for scope mismatch"
    );
    let stderr_lc = result.stderr.to_ascii_lowercase();
    assert!(
        stderr_lc.contains("denied") || stderr_lc.contains("failed"),
        "stderr should indicate request was denied, got: {}",
        result.stderr
    );
}

// -----------------------------------------------------------------------
// Error: double-attach — attaching a second credential to the same req
// -----------------------------------------------------------------------

#[tokio::test]
async fn double_attach_rejected() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .register_credential("cred_a", cred(&base_url, "token-a"))
            .expect("register_credential failed");

        sandbox
            .register_credential("cred_b", cred(&base_url, "token-b"))
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        // Guest code uses attach_credential directly to attempt double-attach
        let code = format!(
            r#"
import wit_world.imports.wasi_http_types as http_types
import wit_world.imports.credentials as creds

fields = http_types.Fields.from_list([("user-agent", b"test")])
req = http_types.OutgoingRequest(fields)
req.set_method(http_types.Method_Get())
req.set_scheme(http_types.Scheme_Http())
req.set_authority("{authority}")
req.set_path_with_query("/double")

# First attach — should succeed
creds.attach(req, "cred_a")

# Second attach — should fail with already-attached
try:
    creds.attach(req, "cred_b")
    print("ERROR: second attach did not raise")
except Exception as e:
    print(f"OK: {{e}}")
"#,
            authority = base_url.trim_start_matches("http://").trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("OK:"),
        "expected OK from double-attach rejection, got stdout: {}",
        result.stdout
    );
}

// -----------------------------------------------------------------------
// Security: guest-set Authorization header is replaced by credential
// -----------------------------------------------------------------------

#[tokio::test]
async fn guest_cannot_override_credential_header() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .register_credential("legit", cred(&base_url, "real-token"))
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        // Guest manually sets Authorization header, then also attaches a
        // credential.  The host must strip the guest's header and inject
        // the credential's value instead.
        let code = format!(
            r#"
import wit_world.imports.wasi_http_types as http_types
import wit_world.imports.outgoing_handler as outgoing_handler
import wit_world.imports.credentials as creds

fields = http_types.Fields.from_list([
    ("user-agent", b"test"),
    ("authorization", b"Bearer evil-guest-token"),
])
req = http_types.OutgoingRequest(fields)
req.set_method(http_types.Method_Get())
req.set_scheme(http_types.Scheme_Http())
req.set_authority("{authority}")
req.set_path_with_query("/sneaky")

creds.attach(req, "legit")

future_resp = outgoing_handler.handle(req, None)
pollable = future_resp.subscribe()
pollable.block()
resp_result = future_resp.get()
resp = resp_result
if hasattr(resp, 'value'):
    resp = resp.value
if hasattr(resp, 'value'):
    resp = resp.value
import json
body_stream = resp.consume().stream()
chunks = []
while True:
    try:
        chunk = body_stream.read(65536)
        if chunk:
            chunks.append(chunk)
        else:
            break
    except:
        break
print(b"".join(chunks).decode())
"#,
            authority = base_url.trim_start_matches("http://").trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    let headers = echo["headers"].as_object().expect("missing headers");
    let auth_val = headers
        .get("authorization")
        .and_then(|v| v.as_str())
        .expect("authorization header missing");

    assert_eq!(
        auth_val, "Bearer real-token",
        "credential header must be the host-injected value, not the guest's"
    );
    assert!(
        !auth_val.contains("evil"),
        "guest's fake authorization header must be stripped"
    );
}

// -----------------------------------------------------------------------
// No credential attached — request goes through without auth header
// -----------------------------------------------------------------------

#[tokio::test]
async fn no_credential_means_no_auth_header() {
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
            .expect("allow_domain failed");

        let code = format!(
            r#"
resp = http_get("{base_url}/open")
print(resp["body"])
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);

    let echo: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).expect("failed to parse echo response");
    let headers = echo["headers"].as_object().expect("missing headers");
    assert!(
        headers.get("authorization").is_none(),
        "no credential attached — authorization header should be absent"
    );
}

// -----------------------------------------------------------------------
// Host-side duplicate registration is rejected
// -----------------------------------------------------------------------

#[tokio::test]
async fn duplicate_credential_registration_rejected() {
    let sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(python_guest_path())
        .build()
        .expect("failed to create sandbox");

    sandbox
        .register_credential("dup", cred("https://example.com", "tok"))
        .expect("first registration should succeed");

    let err = sandbox
        .register_credential("dup", cred("https://example.com", "tok2"))
        .expect_err("second registration should fail");

    assert!(
        format!("{err}").contains("already registered"),
        "error should mention 'already registered', got: {err}"
    );
}
