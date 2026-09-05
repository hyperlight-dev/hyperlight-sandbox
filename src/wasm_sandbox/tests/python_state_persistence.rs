//! Integration test: Python guest module globals persist across `run()`.
//!
//! This test pins the documented contract that the Python `Executor`
//! reuses one module-level namespace for every call to `run()` on the
//! same sandbox instance. The previous implementation built a fresh
//! `globals` dict on every call (`exec(code, {...})`), which silently
//! discarded any `def`, `class`, or top-level assignment between runs.
//! That contradicted:
//!
//!   * the `WasmSandbox` `snapshot`/`restore` contract — the documented
//!     mechanism for rewinding guest state — which only makes sense
//!     if state otherwise survives a `run()` boundary;
//!   * the `python_basics` example's "state was rolled back" narrative;
//!   * the JavaScript guest, which preserves `globalThis` across runs.
//!
//! The tests below would have failed on the prior implementation; they
//! pass once `Executor` stores its globals on the instance and reuses
//! them across `run()` calls.

use std::path::Path;

use hyperlight_sandbox::SandboxBuilder;
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

/// A `def` at module top level in `run()` #1 must be callable in `run()` #2.
#[tokio::test]
async fn python_function_definition_persists_across_runs() {
    let result = tokio::task::spawn_blocking(|| {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox
            .run("def word_count(text): return len(text.split())")
            .expect("first run failed");

        sandbox
            .run("print(word_count('hello world from hyperlight'))")
            .expect("second run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "4");
}

/// A bare module-level assignment in `run()` #1 must be readable in `run()` #2.
#[tokio::test]
async fn python_top_level_assignment_persists_across_runs() {
    let result = tokio::task::spawn_blocking(|| {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        sandbox.run("counter = 100").expect("first run failed");
        sandbox
            .run("print(f'counter = {counter}')")
            .expect("second run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "counter = 100");
}

/// `snapshot` + `restore` must continue to rewind the persistent
/// namespace, undoing any names defined since the snapshot. This is
/// the contract documented on `WasmSandbox`; the persistence fix must
/// not regress it.
#[tokio::test]
async fn python_restore_rewinds_module_globals() {
    let result = tokio::task::spawn_blocking(|| {
        let mut sandbox = SandboxBuilder::new()
            .guest(Wasm)
            .module_path(python_guest_path())
            .build()
            .expect("failed to create sandbox");

        let snap = sandbox.snapshot().expect("snapshot failed");
        sandbox
            .run("rolled_back = 'still here'")
            .expect("set failed");
        sandbox.restore(&snap).expect("restore failed");

        sandbox
            .run(
                r#"
try:
    print(rolled_back)
except NameError:
    print("rolled_back is undefined")
"#,
            )
            .expect("post-restore run failed")
    })
    .await
    .unwrap();

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "rolled_back is undefined");
}
