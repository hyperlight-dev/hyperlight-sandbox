//! End-to-end tests for filesystem quotas enforced across the guest WASI boundary.

use std::path::Path;

use hyperlight_sandbox::{DirPerms, FilePerms, FilesystemLimits, SandboxBuilder};
use hyperlight_wasm_sandbox::Wasm;

fn python_guest_path() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("guests/python/python-sandbox.aot")
        .display()
        .to_string()
}

fn quota_sandbox(
    max_file_size: u64,
    max_total_size: u64,
    max_file_count: usize,
) -> hyperlight_sandbox::Sandbox<Wasm> {
    SandboxBuilder::new()
        .guest(Wasm)
        .module_path(python_guest_path())
        .temp_output()
        .filesystem_limits(
            FilesystemLimits::new(max_file_size, max_total_size, max_file_count)
                .expect("invalid filesystem limits"),
        )
        .build()
        .expect("failed to create sandbox")
}

#[test]
fn guest_wasi_write_within_quota_succeeds() {
    let mut sandbox = quota_sandbox(4, 4, 1);

    let result = sandbox
        .run(
            r#"
with open('/output/allowed.bin', 'wb') as file:
    file.write(b'abcd')
"#,
        )
        .expect("sandbox run failed");

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");
    assert_eq!(
        std::fs::read(output_dir.join("allowed.bin")).expect("failed to read guest output"),
        b"abcd"
    );
}

#[test]
fn output_persists_across_runs() {
    let output = tempfile::tempdir().expect("failed to create output directory");
    std::fs::write(output.path().join("host-staged.txt"), b"from host")
        .expect("failed to stage host output");
    let mut sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(python_guest_path())
        .output_dir(
            output.path(),
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .build()
        .expect("failed to create sandbox");

    let first = sandbox
        .run(
            r#"
with open('/output/host-staged.txt') as file:
    print(file.read())
with open('/output/guest-created.txt', 'w') as file:
    file.write('from guest')
"#,
        )
        .expect("first sandbox run failed");
    assert_eq!(first.exit_code, 0, "stderr: {}", first.stderr);
    assert_eq!(first.stdout.trim(), "from host");

    let second = sandbox
        .run(
            r#"
with open('/output/guest-created.txt') as file:
    print(file.read())
"#,
        )
        .expect("second sandbox run failed");
    assert_eq!(second.exit_code, 0, "stderr: {}", second.stderr);
    assert_eq!(second.stdout.trim(), "from guest");
}

#[test]
fn restore_explicitly_clears_persisted_output() {
    let mut sandbox = quota_sandbox(32, 64, 4);
    let snapshot = sandbox.snapshot().expect("failed to snapshot sandbox");
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");

    let result = sandbox
        .run(
            r#"
with open('/output/persisted.txt', 'w') as file:
    file.write('persisted')
"#,
        )
        .expect("sandbox run failed");
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(output_dir.join("persisted.txt").exists());

    sandbox
        .restore(&snapshot)
        .expect("failed to restore sandbox");

    assert!(!output_dir.join("persisted.txt").exists());
    assert!(
        sandbox
            .get_output_files()
            .expect("failed to list output files")
            .is_empty()
    );
}

#[test]
fn host_changes_over_quota_fail_before_guest_entry() {
    let mut sandbox = quota_sandbox(4, 4, 1);
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");
    std::fs::write(output_dir.join("host-staged.bin"), b"12345")
        .expect("failed to stage over-quota output");

    let error = sandbox
        .run("print('guest entered')")
        .expect_err("over-quota output should reject the run");

    assert!(
        error
            .to_string()
            .contains("file exceeds logical size limit"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn guest_wasi_write_over_quota_is_rejected_before_mutation() {
    let mut sandbox = quota_sandbox(4, 4, 1);

    let result = sandbox
        .run(
            r#"
with open('/output/rejected.bin', 'wb') as file:
    file.write(b'abcde')
"#,
        )
        .expect("sandbox run failed");

    assert_ne!(result.exit_code, 0, "quota-violating guest write succeeded");
    assert!(
        result.stderr.contains("OSError") && result.stderr.contains("I/O error"),
        "unexpected stderr: {}",
        result.stderr
    );
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");
    assert_eq!(
        std::fs::metadata(output_dir.join("rejected.bin"))
            .expect("guest should have created the output file before writing")
            .len(),
        0,
        "quota rejection mutated the output file"
    );
}

#[test]
fn guest_wasi_sparse_write_within_quota_succeeds() {
    let mut sandbox = quota_sandbox(4, 4, 1);

    let result = sandbox
        .run(
            r#"
with open('/output/allowed-sparse.bin', 'wb') as file:
    file.seek(3)
    file.write(b'x')
"#,
        )
        .expect("sandbox run failed");

    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");
    assert_eq!(
        std::fs::read(output_dir.join("allowed-sparse.bin"))
            .expect("failed to read sparse guest output"),
        b"\0\0\0x"
    );
}

#[test]
fn guest_wasi_sparse_write_over_quota_is_rejected_before_mutation() {
    let mut sandbox = quota_sandbox(4, 4, 1);

    let result = sandbox
        .run(
            r#"
with open('/output/rejected-sparse.bin', 'wb') as file:
    # Seek past EOF to create a four-byte logical hole before writing one byte.
    file.seek(4)
    file.write(b'x')
"#,
        )
        .expect("sandbox run failed");

    assert_ne!(
        result.exit_code, 0,
        "quota-violating sparse write succeeded"
    );
    assert!(
        result.stderr.contains("OSError") && result.stderr.contains("I/O error"),
        "unexpected stderr: {}",
        result.stderr
    );
    let output_dir = sandbox
        .output_path()
        .expect("output path failed")
        .expect("missing output path");
    assert_eq!(
        std::fs::metadata(output_dir.join("rejected-sparse.bin"))
            .expect("guest should have created the sparse output file before writing")
            .len(),
        0,
        "sparse quota rejection mutated the output file"
    );
}
