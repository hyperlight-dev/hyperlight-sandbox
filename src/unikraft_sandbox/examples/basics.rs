//! Basics demo for the hyperlight-unikraft sandbox backend.
//!
//! Exercises: CPython with numpy/pandas, state that persists across runs,
//! tool dispatch, /output files, errors, and snapshot/restore, with timings.
//!
//! Needs the `agent` rootfs. Pass its path as the first argument, or let
//! the example find the copy the `hluk` CLI caches:
//!
//! ```sh
//! cargo run --release -p hyperlight-unikraft-sandbox --example basics [ROOTFS.cpio]
//! ```

use std::time::Instant;

use hyperlight_sandbox::{SandboxBuilder, ToolRegistry};
use hyperlight_unikraft_sandbox::{Unikraft, cached_rootfs};
use serde::Deserialize;

fn separator(title: &str) {
    println!("\n{}", "═".repeat(60));
    println!("{title}");
    println!("{}", "═".repeat(60));
}

#[derive(Deserialize)]
struct MathArgs {
    a: f64,
    b: f64,
}

#[derive(Deserialize)]
struct FetchArgs {
    table: String,
}

fn main() -> anyhow::Result<()> {
    let rootfs = match std::env::args().nth(1) {
        Some(path) => path.into(),
        None => cached_rootfs("agent")?,
    };

    let mut tools = ToolRegistry::new();
    tools.register_typed::<MathArgs, _>("multiply", |args| Ok(serde_json::json!(args.a * args.b)));
    tools.register_typed::<FetchArgs, _>("fetch_data", |args| match args.table.as_str() {
        "sales" => Ok(serde_json::json!([
            {"region": "north", "units": 120, "price": 9.5},
            {"region": "south", "units": 80, "price": 11.0},
            {"region": "north", "units": 45, "price": 9.5},
            {"region": "east", "units": 200, "price": 7.25},
        ])),
        other => anyhow::bail!("no table named {other:?}"),
    });

    separator("Boot");
    let started = Instant::now();
    let mut sandbox = SandboxBuilder::new()
        .module_path(rootfs.display().to_string())
        .with_tools(tools)
        .temp_output()
        .guest(Unikraft::new())
        .build()?;
    println!("booted {} in {:?}", rootfs.display(), started.elapsed());

    separator("Test 1: numpy + pandas");
    let started = Instant::now();
    let result = sandbox.run(
        r#"
import sys, numpy as np, pandas as pd
print(f"Python {sys.version.split()[0]}, numpy {np.__version__}, pandas {pd.__version__}")
a = np.arange(1, 10).reshape(3, 3)
print("det(a + I) =", round(float(np.linalg.det(a + np.eye(3))), 3))
"#,
    )?;
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    println!("({:?})", started.elapsed());
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("numpy"));

    separator("Test 2: state persists across runs");
    sandbox.run("counter = 41")?;
    let started = Instant::now();
    let result = sandbox.run("counter += 1\nprint('counter =', counter)")?;
    print!("{}", result.stdout);
    println!("({:?})", started.elapsed());
    assert!(result.stdout.contains("counter = 42"));

    separator("Test 3: tools + /output");
    let started = Instant::now();
    let result = sandbox.run(
        r#"
import pandas as pd
df = pd.DataFrame(call_tool('fetch_data', table='sales'))
df["revenue"] = df["units"] * df["price"]
summary = df.groupby("region")["revenue"].sum().sort_values(ascending=False)
print(summary.to_string())
print("6 x 7 =", call_tool('multiply', a=6, b=7))
summary.to_csv("/output/summary.csv")
try:
    call_tool('fetch_data', table='nope')
except RuntimeError as e:
    print("tool error surfaced:", e)
"#,
    )?;
    print!("{}", result.stdout);
    eprint!("{}", result.stderr);
    println!("({:?})", started.elapsed());
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("6 x 7 = 42"));
    println!("output files: {:?}", sandbox.get_output_files()?);

    separator("Test 4: errors land in stderr with an exit code");
    let result = sandbox.run("1 / 0")?;
    print!("{}", result.stderr);
    assert_eq!(result.exit_code, 1);
    assert!(result.stderr.contains("ZeroDivisionError"));

    separator("Test 5: snapshot / restore");
    let started = Instant::now();
    let snapshot = sandbox.snapshot()?;
    println!("snapshot: {:?}", started.elapsed());
    sandbox.run("counter = 100\nimport sklearn")?;
    let started = Instant::now();
    sandbox.restore(&snapshot)?;
    println!("restore:  {:?}", started.elapsed());
    let result = sandbox.run("print('counter =', counter)")?;
    print!("{}", result.stdout);
    assert!(result.stdout.contains("counter = 42"));

    separator("All tests passed!");
    Ok(())
}
