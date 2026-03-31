//! Filesystem capabilities demo: exercises all input/output combinations.
//!
//! Run with: `just wasm::sandbox-filesystem-example`

use std::path::Path;

use hyperlight_sandbox::{DirPerms, FilePerms, Sandbox};
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

fn separator(label: &str) {
    println!("\n── {label} ──");
}

fn main() {
    // ── 1: No filesystem ────────────────────────────────────────────
    separator("Test 1: No filesystem");
    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .build()
        .expect("failed to create sandbox without filesystem");

    let result = sandbox.run("print('no filesystem needed')").unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("no filesystem needed"),
        "stdout: {:?}",
        result.stdout,
    );
    let outputs = sandbox.get_output_files().unwrap();
    assert!(outputs.is_empty(), "outputs should be empty");
    println!("OK: sandbox runs without any filesystem preopens");

    // ── 2: Input only ───────────────────────────────────────────────
    separator("Test 2: Input only (read-only preopen, no output)");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("greeting.txt"), b"hello from host").unwrap();

    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .input_dir(input_tmp.path())
        .build()
        .expect("failed to create sandbox with input only");

    let result = sandbox
        .run(
            r#"
with open('/input/greeting.txt') as f:
    content = f.read()
print(f"content: {content}")
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello from host"),
        "stdout: {:?}",
        result.stdout,
    );
    let outputs = sandbox.get_output_files().unwrap();
    assert!(outputs.is_empty(), "no output dir configured");
    println!("OK: guest reads host-provided input files");

    // ── 3: Temp output only ─────────────────────────────────────────
    separator("Test 3: Temp output only (writable preopen, no input)");
    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .temp_output()
        .build()
        .expect("failed to create sandbox with temp output");

    let result = sandbox
        .run(
            r#"
with open('/output/result.txt', 'w') as f:
    f.write('computed result')
print('wrote output')
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("wrote output"),
        "stdout: {:?}",
        result.stdout,
    );
    let output_dir = sandbox.output_path().unwrap().unwrap();
    let output_files = sandbox.get_output_files().unwrap();
    assert!(output_files.contains(&"result.txt".to_string()));
    assert_eq!(
        std::fs::read(output_dir.join("result.txt")).unwrap(),
        b"computed result",
    );
    println!("OK: guest writes to temp output, host collects files");

    // ── 4: Input + temp output ──────────────────────────────────────
    separator("Test 4: Input + temp output");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("data.json"), br#"{"value": 42}"#).unwrap();

    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .input_dir(input_tmp.path())
        .temp_output()
        .build()
        .expect("failed to create sandbox with input + temp output");

    let result = sandbox
        .run(
            r#"
import json
with open('/input/data.json') as f:
    data = json.load(f)
result = data['value'] * 2
with open('/output/doubled.txt', 'w') as f:
    f.write(str(result))
print(f"doubled: {result}")
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("doubled: 84"),
        "stdout: {:?}",
        result.stdout,
    );
    let output_dir = sandbox.output_path().unwrap().unwrap();
    let output_files = sandbox.get_output_files().unwrap();
    assert!(output_files.contains(&"doubled.txt".to_string()));
    assert_eq!(
        std::fs::read(output_dir.join("doubled.txt")).unwrap(),
        b"84",
    );
    println!("OK: guest reads input, writes output, host collects");

    // ── 5: Explicit output dir ──────────────────────────────────────
    separator("Test 5: Input + explicit output dir with permissions");
    let input_tmp = tempfile::tempdir().unwrap();
    let output_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("source.txt"), b"transform me").unwrap();

    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .input_dir(input_tmp.path())
        .output_dir(
            output_tmp.path(),
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .build()
        .expect("failed to create sandbox with explicit output dir");

    let result = sandbox
        .run(
            r#"
with open('/input/source.txt') as f:
    text = f.read()
with open('/output/upper.txt', 'w') as f:
    f.write(text.upper())
print(f"transformed: {text.upper()}")
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("TRANSFORM ME"),
        "stdout: {:?}",
        result.stdout,
    );
    // Verify via get_output_files
    let output_files = sandbox.get_output_files().unwrap();
    assert!(output_files.contains(&"upper.txt".to_string()));
    // Also verify the file exists on the host filesystem
    let host_file = std::fs::read_to_string(output_tmp.path().join("upper.txt")).unwrap();
    assert_eq!(host_file, "TRANSFORM ME");
    println!("OK: output written to explicit dir, visible on host filesystem");

    // ── 6: Output is wiped between runs ─────────────────────────────
    separator("Test 6: Output is ephemeral (wiped between runs)");
    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .temp_output()
        .build()
        .expect("failed to create sandbox");

    sandbox
        .run(
            r#"
with open('/output/run1.txt', 'w') as f:
    f.write('first run')
"#,
        )
        .unwrap();
    let first = sandbox.get_output_files().unwrap();
    assert!(
        first.contains(&"run1.txt".to_string()),
        "run1.txt should exist after first run"
    );

    let result = sandbox
        .run(
            r#"
with open('/output/run2.txt', 'w') as f:
    f.write('second run')
print('wrote run2')
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let second = sandbox.get_output_files().unwrap();
    assert!(
        !second.contains(&"run1.txt".to_string()),
        "run1.txt should be wiped"
    );
    assert!(
        second.contains(&"run2.txt".to_string()),
        "run2.txt should exist"
    );
    println!("OK: output files wiped between runs");

    // ── 7: Input is read-only (guest cannot write) ──────────────────
    separator("Test 7: Input is read-only");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("readonly.txt"), b"do not modify").unwrap();

    let mut sandbox = Sandbox::builder()
        .guest(Wasm)
        .module_path(python_guest_path())
        .input_dir(input_tmp.path())
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(
            r#"
try:
    with open('/input/readonly.txt', 'w') as f:
        f.write('hacked')
    print('FAIL: write succeeded')
except (OSError, PermissionError) as e:
    print(f'OK: write blocked: {e}')
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("OK: write blocked"),
        "guest should not be able to write to /input, got: {:?}",
        result.stdout,
    );
    // Verify host file is untouched
    let content = std::fs::read_to_string(input_tmp.path().join("readonly.txt")).unwrap();
    assert_eq!(content, "do not modify");
    println!("OK: guest cannot write to input directory");

    println!("\n✅ All filesystem tests passed!");
}
