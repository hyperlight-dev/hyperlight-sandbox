//! Integration test: Python guest module globals persist across `run()`.
//!
//! This test pins the documented contract that the Python `Executor`
//! reuses one module-level namespace for every call to `run()` on the
//! same sandbox instance. It tests:
//!
//!   * the `WasmSandbox` `snapshot`/`restore` resets state — the documented
//!     mechanism for rewinding guest state
//!   * if state otherwise survives a `run()` boundary
//!   * behaves like the JavaScript guest, which preserves `globalThis` across runs
//!

use std::path::Path;

use hyperlight_sandbox::SandboxBuilder;
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

/// A `def` at module top level in `run()` must be callable in the second `run()`
#[test]
fn python_function_definition_persists_across_runs() {
    let mut sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(python_guest_path())
        .build()
        .expect("failed to create sandbox");

    let snap = sandbox.snapshot().expect("snapshot failed");
    sandbox
        .run("def word_count(text): return len(text.split())")
        .expect("first run failed");

    let persist_result = sandbox
        .run("print(word_count('hello world from hyperlight'))")
        .expect("second run failed");

    sandbox.restore(&snap).expect("restore failed");
    let reset_result = sandbox
        .run(
            r#"
try:
    print(word_count('hello world from hyperlight'))
except NameError:
    print("word_count is undefined")
"#,
        )
        .expect("post-restore run failed");

    assert_eq!(
        persist_result.exit_code, 0,
        "stderr: {}",
        persist_result.stderr
    );
    assert_eq!(persist_result.stdout.trim(), "4");
    assert_eq!(reset_result.exit_code, 0, "stderr: {}", reset_result.stderr);
    assert_eq!(reset_result.stdout.trim(), "word_count is undefined");
}

/// A bare module-level assignment in `run()` must be readable in the second `run()`
#[test]
fn python_top_level_assignment_persists_across_runs() {
    let mut sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(python_guest_path())
        .build()
        .expect("failed to create sandbox");

    let snap = sandbox.snapshot().expect("snapshot failed");
    sandbox.run("counter = 100").expect("first run failed");
    let persist_result = sandbox
        .run("print(f'counter = {counter}')")
        .expect("second run failed");

    sandbox.restore(&snap).expect("restore failed");
    let reset_result = sandbox
        .run(
            r#"
try:
    print(f'counter = {counter}')
except NameError:
    print("counter is undefined")
"#,
        )
        .expect("post-restore run failed");

    assert_eq!(
        persist_result.exit_code, 0,
        "stderr: {}",
        persist_result.stderr
    );
    assert_eq!(persist_result.stdout.trim(), "counter = 100");
    assert_eq!(reset_result.exit_code, 0, "stderr: {}", reset_result.stderr);
    assert_eq!(reset_result.stdout.trim(), "counter is undefined");
}

/// `snapshot` + `restore` must continue to rewind the persistent
/// namespace, undoing any names defined since the snapshot. This is
/// the contract documented on `WasmSandbox`; the persistence fix must
/// not regress it.
#[test]
fn python_restore_rewinds_module_globals() {
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

    let result = sandbox
        .run(
            r#"
try:
    print(rolled_back)
except NameError:
    print("rolled_back is undefined")
"#,
        )
        .expect("post-restore run failed");

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "rolled_back is undefined");
}
