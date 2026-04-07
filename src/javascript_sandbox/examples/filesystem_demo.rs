//! Filesystem capabilities demo for the JavaScript sandbox.

use hyperlight_javascript_sandbox::HyperlightJs;
use hyperlight_sandbox::{DirPerms, FilePerms, SandboxBuilder};

fn separator(label: &str) {
    println!("\n── {label} ──");
}

fn main() {
    // ── 1: No filesystem ────────────────────────────────────────────
    separator("Test 1: No filesystem");
    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .build()
        .expect("failed to create sandbox");

    let result = sandbox.run("console.log('no filesystem');").unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("no filesystem"),
        "stdout: {:?}",
        result.stdout,
    );
    let outputs = sandbox.get_output_files().unwrap();
    assert!(outputs.is_empty());
    println!("OK: runs without filesystem");

    // ── 2: Input only ───────────────────────────────────────────────
    separator("Test 2: Input only");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("hello.txt"), b"hello from host").unwrap();

    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .input_dir(input_tmp.path())
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(r#"const text = read_file('/input/hello.txt'); console.log('read: ' + text);"#)
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello from host"),
        "stdout: {:?}",
        result.stdout,
    );
    println!("OK: guest reads input");

    // ── 3: Temp output only ─────────────────────────────────────────
    separator("Test 3: Temp output only");
    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .temp_output()
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(r#"write_file('/output/result.txt', 'js output'); console.log('wrote');"#)
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let output_dir = sandbox.output_path().unwrap().unwrap();
    let output_files = sandbox.get_output_files().unwrap();
    assert!(output_files.contains(&"result.txt".to_string()));
    assert_eq!(
        std::fs::read(output_dir.join("result.txt")).unwrap(),
        b"js output",
    );
    println!("OK: guest writes to temp output");

    // ── 4: Input + temp output ──────────────────────────────────────
    separator("Test 4: Input + temp output");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("data.json"), br#"{"n": 21}"#).unwrap();

    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .input_dir(input_tmp.path())
        .temp_output()
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(
            r#"
const raw = read_file('/input/data.json');
const data = JSON.parse(raw);
write_file('/output/doubled.txt', String(data.n * 2));
console.log('doubled: ' + data.n * 2);
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("doubled: 42"),
        "stdout: {:?}",
        result.stdout
    );
    let output_dir = sandbox.output_path().unwrap().unwrap();
    let output_files = sandbox.get_output_files().unwrap();
    assert!(output_files.contains(&"doubled.txt".to_string()));
    assert_eq!(
        std::fs::read(output_dir.join("doubled.txt")).unwrap(),
        b"42",
    );
    println!("OK: reads input, writes output");

    // ── 5: Explicit output dir ──────────────────────────────────────
    separator("Test 5: Explicit output dir");
    let input_tmp = tempfile::tempdir().unwrap();
    let output_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("msg.txt"), b"shout").unwrap();

    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .input_dir(input_tmp.path())
        .output_dir(
            output_tmp.path(),
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(
            r#"
const text = read_file('/input/msg.txt');
write_file('/output/upper.txt', text.toUpperCase());
console.log('done');
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    let host_file = std::fs::read_to_string(output_tmp.path().join("upper.txt")).unwrap();
    assert_eq!(host_file, "SHOUT");
    println!("OK: output visible on host filesystem");

    // ── 6: Output wiped between runs ────────────────────────────────
    separator("Test 6: Output is ephemeral");
    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .temp_output()
        .build()
        .expect("failed to create sandbox");

    sandbox
        .run(r#"write_file('/output/run1.txt', 'first');"#)
        .unwrap();
    let first = sandbox.get_output_files().unwrap();
    assert!(first.contains(&"run1.txt".to_string()));

    let result = sandbox
        .run(r#"write_file('/output/run2.txt', 'second'); console.log('ok');"#)
        .unwrap();
    assert_eq!(result.exit_code, 0);
    let second = sandbox.get_output_files().unwrap();
    assert!(
        !second.contains(&"run1.txt".to_string()),
        "run1 should be wiped"
    );
    assert!(second.contains(&"run2.txt".to_string()));
    println!("OK: output wiped between runs");

    // ── 7: Input is read-only ───────────────────────────────────────
    separator("Test 7: Input is read-only");
    let input_tmp = tempfile::tempdir().unwrap();
    std::fs::write(input_tmp.path().join("readonly.txt"), b"do not modify").unwrap();

    let mut sandbox = SandboxBuilder::new()
        .guest(HyperlightJs)
        .input_dir(input_tmp.path())
        .build()
        .expect("failed to create sandbox");

    let result = sandbox
        .run(
            r#"
try {
    write_file('/input/readonly.txt', 'hacked');
    console.log('FAIL: write succeeded');
} catch (e) {
    console.log('OK: write blocked: ' + e);
}
"#,
        )
        .unwrap();
    assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("OK: write blocked"),
        "guest should not write to /input, got: {:?}",
        result.stdout,
    );
    let content = std::fs::read_to_string(input_tmp.path().join("readonly.txt")).unwrap();
    assert_eq!(content, "do not modify");
    println!("OK: guest cannot write to input");

    println!("\n✅ All JS filesystem tests passed!");
}
