use std::path::Path;

use hyperlight_sandbox::SandboxBuilder;
use hyperlight_sandbox_wasm::Wasm;

#[test]
fn large_source_runs_without_exhausting_h2g_descriptors() {
    let module_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/python/python-sandbox.aot");
    let mut sandbox = SandboxBuilder::new()
        .guest(Wasm)
        .module_path(module_path.display().to_string())
        .build()
        .expect("failed to create sandbox");

    for source_size in [256 * 1024, 1024 * 1024] {
        let suffix = "\nprint('large source accepted')\n";
        let code = format!("#{}{}", "x".repeat(source_size - 1 - suffix.len()), suffix);
        let result = sandbox.run(&code).expect("sandbox run failed");

        assert_eq!(
            result.exit_code, 0,
            "{source_size}-byte source failed: {}",
            result.stderr
        );
        assert_eq!(result.stdout.trim(), "large source accepted");
    }
}
