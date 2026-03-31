//! Example: run JavaScript and Python code in a Nanvix microkernel sandbox.

use hyperlight_nanvix_sandbox::{NanvixJavaScript, NanvixPython};
use hyperlight_sandbox::Sandbox;

fn main() {
    // --- JavaScript ---
    println!("=== Nanvix JavaScript Sandbox ===\n");
    let mut js_sandbox = match Sandbox::builder()
        .guest(NanvixJavaScript)
        .build()
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create JS sandbox: {e:#}");
            std::process::exit(1);
        }
    };

    println!("--- Test 1: Basic JS execution ---");
    match js_sandbox.run(
        r#"
console.log("Hello from JavaScript in Nanvix!");
console.log("2 + 3 =", 2 + 3);
"#,
    ) {
        Ok(result) => {
            assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
            assert!(
                result.stdout.contains("Hello from JavaScript in Nanvix!"),
                "stdout was {:?}",
                result.stdout
            );
            assert!(
                result.stdout.contains("2 + 3 = 5"),
                "stdout was {:?}",
                result.stdout
            );
            println!("stdout: {:?}", result.stdout);
            println!("exit_code: {}", result.exit_code);
        }
        Err(e) => panic!("Test 1 failed: {e}"),
    }

    // --- Python ---
    println!("\n=== Nanvix Python Sandbox ===\n");
    let mut py_sandbox = match Sandbox::builder()
        .guest(NanvixPython)
        .build()
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create Python sandbox: {e:#}");
            std::process::exit(1);
        }
    };

    println!("--- Test 2: Basic Python execution ---");
    match py_sandbox.run(
        r#"
print("Hello from Python in Nanvix!")
print(f"2 + 3 = {2 + 3}")
"#,
    ) {
        Ok(result) => {
            assert_eq!(result.exit_code, 0, "stderr: {}", result.stderr);
            assert!(
                result.stdout.contains("Hello from Python in Nanvix!"),
                "stdout was {:?}",
                result.stdout
            );
            assert!(
                result.stdout.contains("2 + 3 = 5"),
                "stdout was {:?}",
                result.stdout
            );
            println!("stdout: {:?}", result.stdout);
            println!("exit_code: {}", result.exit_code);
        }
        Err(e) => panic!("Test 2 failed: {e}"),
    }

    // --- Snapshot (unsupported) ---
    println!("\n--- Test 3: Snapshot (expected to fail) ---");
    match js_sandbox.snapshot() {
        Ok(_) => panic!("Snapshot should not succeed on Nanvix!"),
        Err(e) => println!("Snapshot correctly unsupported: {e}"),
    }

    println!("\n\u{2705} All tests passed!");
}
