//! A relative output directory stays the one the sandbox was built with
//! when the process changes its working directory.
//!
//! On its own in this binary: the working directory is the process's, and
//! the other tests run on threads of theirs. Needs KVM and the cached
//! `python` image, as `python_image.rs` does; skips without the image.

use hyperlight_sandbox::{DirPerms, FilePerms, SandboxBuilder};
use hyperlight_unikraft_sandbox::{Unikraft, cached_rootfs};

#[test]
fn relative_output_dir_ignores_later_chdir() {
    let rootfs = match cached_rootfs("python") {
        Ok(path) => path,
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let base = tempfile::tempdir().unwrap();
    let (a, b) = (base.path().join("a"), base.path().join("b"));
    std::fs::create_dir_all(a.join("out")).unwrap();
    // Same relative path under another directory, with data to keep.
    std::fs::create_dir_all(b.join("out/userdata")).unwrap();
    std::fs::write(b.join("out/userdata/precious.txt"), "keep").unwrap();

    std::env::set_current_dir(&a).unwrap();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs.display().to_string())
        .output_dir(
            "out",
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .guest(Unikraft::new().native_network(true))
        .build()
        .unwrap();
    std::env::set_current_dir(&b).unwrap();

    let r = sandbox
        .run("open('/output/x.txt', 'w').write('x')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert!(a.join("out/x.txt").is_file());
    // The clear before the next run, and a symlink the guest leaves
    // (which fails the run and is removed), act on a/out alone.
    let r = sandbox
        .run("import os\nos.symlink('x', '/output/link')")
        .unwrap();
    assert_eq!(r.exit_code, 1, "{}", r.stderr);
    assert!(!a.join("out/x.txt").exists());
    assert_eq!(std::fs::read_dir(a.join("out")).unwrap().count(), 0);
    assert_eq!(
        std::fs::read_to_string(b.join("out/userdata/precious.txt")).unwrap(),
        "keep"
    );
    // A change to the allowlist re-boots the guest, remounting /output:
    // still a/out.
    sandbox
        .allow_domain("http://127.0.0.1", hyperlight_sandbox::MethodFilter::Any)
        .unwrap();
    let r = sandbox
        .run("open('/output/y.txt', 'w').write('y')")
        .unwrap();
    assert_eq!(r.exit_code, 0, "{}", r.stderr);
    assert!(a.join("out/y.txt").is_file());
    assert!(!b.join("out/y.txt").exists());
}
