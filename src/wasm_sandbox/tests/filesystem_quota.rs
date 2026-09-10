//! End-to-end tests for filesystem quotas enforced across the guest WASI boundary.

use std::path::Path;

use hyperlight_sandbox::{FilesystemLimits, SandboxBuilder};
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
