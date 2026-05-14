//! Integration tests: scoped-credential injection end-to-end.
//!
//! Each test spins up a local [`EchoServer`], registers one or more
//! credentials, then runs guest Python code that exercises the
//! `credential=` kwarg on `http_get`/`http_post`.  The echo server
//! reflects all received headers so we can assert exactly what the
//! host injected (or blocked).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hyperlight_sandbox::test_utils::EchoServer;
use hyperlight_sandbox::{CredentialEntry, HttpMethod, ResolverFn, SandboxBuilder};
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

/// Helper to build a [`CredentialEntry`] with sensible defaults and a
/// static token value.  Tests that need rotation or fault injection
/// build a custom [`ResolverFn`] inline instead.
fn cred(target: &str, token: &str) -> CredentialEntry {
    CredentialEntry::with_static_resolver(target, "authorization", "Bearer ", token)
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

// -----------------------------------------------------------------------
// Resolver is invoked per-request — proves token refresh contract
// -----------------------------------------------------------------------

#[tokio::test]
async fn resolver_invoked_per_request() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        // Resolver that returns a different token on each invocation.
        // This is the canonical proof that the host calls the resolver
        // for every outgoing credentialed request, not just once at
        // registration time.
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_resolver = Arc::clone(&counter);
        let resolver: ResolverFn = Arc::new(move || {
            let n = counter_for_resolver.fetch_add(1, Ordering::SeqCst);
            Ok(format!("rotating-token-{n}"))
        });

        sandbox
            .register_credential(
                "rotating",
                CredentialEntry {
                    target: base_url.clone(),
                    header: "authorization".to_string(),
                    prefix: "Bearer ".to_string(),
                    resolver,
                },
            )
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        let code = format!(
            r#"
import json
r1 = http_get("{base_url}/api/one", credential="rotating")
r2 = http_get("{base_url}/api/two", credential="rotating")
print(json.dumps([json.loads(r1["body"]), json.loads(r2["body"])]))
"#,
            base_url = base_url.trim_end_matches('/')
        );

        (sandbox.run(&code).expect("sandbox run failed"), counter)
    })
    .await
    .unwrap();

    let (exec, counter) = result;
    assert_eq!(exec.exit_code, 0, "stderr: {}", exec.stderr);

    let echoes: Vec<serde_json::Value> =
        serde_json::from_str(exec.stdout.trim()).expect("failed to parse echo array");
    assert_eq!(echoes.len(), 2, "expected two echoed responses");
    assert_eq!(
        echoes[0]["headers"]["authorization"].as_str(),
        Some("Bearer rotating-token-0"),
        "first request should see token-0"
    );
    assert_eq!(
        echoes[1]["headers"]["authorization"].as_str(),
        Some("Bearer rotating-token-1"),
        "second request should see token-1 — resolver MUST be called per request"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "resolver should have been invoked exactly twice"
    );
}

// -----------------------------------------------------------------------
// Resolver failure surfaces as a request error with no token leakage
// -----------------------------------------------------------------------

#[tokio::test]
async fn resolver_failure_surfaces_as_error() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let result = tokio::task::spawn_blocking(move || {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        // Resolver that always fails.  The diagnostic string MUST NOT
        // appear in any guest-visible error — the host redacts it to a
        // fixed message.
        let resolver: ResolverFn =
            Arc::new(|| Err("secret-bearing diagnostic that must not leak".to_string()));

        sandbox
            .register_credential(
                "broken",
                CredentialEntry {
                    target: base_url.clone(),
                    header: "authorization".to_string(),
                    prefix: "Bearer ".to_string(),
                    resolver,
                },
            )
            .expect("register_credential failed");

        sandbox
            .allow_domain(&base_url, vec![HttpMethod::Get])
            .expect("allow_domain failed");

        let code = format!(
            r#"
try:
    resp = http_get("{base_url}/api/data", credential="broken")
    print("UNEXPECTED_OK:" + resp["body"])
except Exception as e:
    print("ERR:" + repr(e))
"#,
            base_url = base_url.trim_end_matches('/')
        );

        sandbox.run(&code).expect("sandbox run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.starts_with("ERR:"),
        "guest should have raised an exception, got stdout: {}",
        result.stdout
    );
    // The host-redacted message is the only thing the guest may see.
    assert!(
        !result.stdout.contains("secret-bearing"),
        "resolver diagnostic must NOT leak to guest, stdout was: {}",
        result.stdout
    );
    assert!(
        !result.stdout.contains("must not leak"),
        "resolver diagnostic must NOT leak to guest, stdout was: {}",
        result.stdout
    );
}

// -----------------------------------------------------------------------
// Multi-tenant isolation: two sandboxes that register the same
// credential id with DIFFERENT tokens must each see only their own
// token.  Proves the credential registry is per-`Sandbox` instance
// — there is no global key table, no shared `Arc`, no cross-instance
// lookup path.
//
// If this test ever fails it means the host has acquired a shared
// registry by mistake (e.g. a `lazy_static!`, a `OnceCell`, or a
// stray `Arc::clone` between sandboxes).  Treat as critical.
// -----------------------------------------------------------------------

#[tokio::test]
async fn isolated_registries_across_sandboxes() {
    let server = EchoServer::start().await;
    let base_url = server.url("");

    let (result_a, result_b) = tokio::task::spawn_blocking(move || {
        // ---- Sandbox A: id="shared" → token-tenant-A ----
        let result_a = {
            let mut sandbox = SandboxBuilder::new()
                .guest(Wasm)
                .module_path(python_guest_path())
                .build()
                .expect("failed to create sandbox A");

            sandbox
                .register_credential("shared", cred(&base_url, "token-tenant-A"))
                .expect("register_credential on sandbox A failed");

            sandbox
                .allow_domain(&base_url, vec![HttpMethod::Get])
                .expect("allow_domain on sandbox A failed");

            let code = format!(
                r#"
resp = http_get("{base_url}/tenant-a", credential="shared")
print(resp["body"])
"#,
                base_url = base_url.trim_end_matches('/')
            );

            sandbox.run(&code).expect("sandbox A run failed")
        };

        // ---- Sandbox B: same id="shared" → token-tenant-B ----
        let result_b = {
            let mut sandbox = SandboxBuilder::new()
                .guest(Wasm)
                .module_path(python_guest_path())
                .build()
                .expect("failed to create sandbox B");

            sandbox
                .register_credential("shared", cred(&base_url, "token-tenant-B"))
                .expect("register_credential on sandbox B failed");

            sandbox
                .allow_domain(&base_url, vec![HttpMethod::Get])
                .expect("allow_domain on sandbox B failed");

            let code = format!(
                r#"
resp = http_get("{base_url}/tenant-b", credential="shared")
print(resp["body"])
"#,
                base_url = base_url.trim_end_matches('/')
            );

            sandbox.run(&code).expect("sandbox B run failed")
        };

        (result_a, result_b)
    })
    .await
    .unwrap();

    assert_eq!(
        result_a.exit_code, 0,
        "sandbox A stderr: {}",
        result_a.stderr
    );
    assert_eq!(
        result_b.exit_code, 0,
        "sandbox B stderr: {}",
        result_b.stderr
    );

    let echo_a: serde_json::Value =
        serde_json::from_str(result_a.stdout.trim()).expect("failed to parse echo response A");
    let echo_b: serde_json::Value =
        serde_json::from_str(result_b.stdout.trim()).expect("failed to parse echo response B");

    assert_eq!(
        echo_a["headers"]["authorization"].as_str(),
        Some("Bearer token-tenant-A"),
        "sandbox A must see ONLY its own token"
    );
    assert_eq!(
        echo_b["headers"]["authorization"].as_str(),
        Some("Bearer token-tenant-B"),
        "sandbox B must see ONLY its own token"
    );

    // Belt-and-braces: neither sandbox's stdout contains the other's
    // token.  Catches any future regression where, say, a debug log
    // path or a shared registry accidentally surfaces the foreign
    // value to the guest.
    assert!(
        !result_a.stdout.contains("token-tenant-B"),
        "sandbox A leaked sandbox B's token: {}",
        result_a.stdout
    );
    assert!(
        !result_b.stdout.contains("token-tenant-A"),
        "sandbox B leaked sandbox A's token: {}",
        result_b.stdout
    );
}
