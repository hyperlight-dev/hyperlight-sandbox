use std::hash::{DefaultHasher, Hasher};

fn main() {
    let mut hash = DefaultHasher::new();
    for path in ["src/protocol.rs", "../../Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
        hash.write(&std::fs::read(path).expect("read protocol build input"));
    }
    println!(
        "cargo:rustc-env=SANDBOX_WORKER_BUILD={:016x}",
        hash.finish()
    );
}
