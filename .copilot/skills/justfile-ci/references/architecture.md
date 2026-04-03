# Justfile & CI Architecture Reference

## Root Justfile Recipe Map

| Recipe | Delegates to | Purpose |
|--------|-------------|---------|
| `build` | `wasm::build`, `jss::build`, `nanvix::build`, `python::build` | Build everything |
| `test` | `test-rust`, `wasm::test`, `python::python-test` | All tests |
| `test-rust` | (direct cargo) | Core crate unit + integration tests |
| `lint` | `lint-rust`, `wasm::lint`, `js::lint`, `python::lint` | All linters |
| `fmt` | `fmt-rust`, `python::fmt` | Format all code |
| `fmt-check` | `fmt-check-rust`, `python::fmt-check` | Check formatting |
| `examples` | `wasm::examples`, `js::examples`, `python::examples` | Run all examples |
| `fuzz` | `python::python-fuzz` | Python fuzz tests |
| `benchmark` | `python::python-sandbox-benchmark` | Python benchmark |
| `clean` | `wasm::clean`, `python::clean` | Clean build artifacts |

## CI Job Layout

```
ci.yml
├── rust           — fmt-check-rust, lint-rust, test-rust
├── wasm-sandbox   — wasm build, lint, test, examples + python fmt-check/lint/build/examples/python-test/fuzz/benchmark/integration-examples
├── javascript-sandbox — js build, lint, test, examples
└── nanvix-sandbox — nanvix build (examples skipped pending upstream)
```

## Subproject Justfile Responsibilities

### wasm (`src/wasm_sandbox/Justfile`)
- Sets `WIT_WORLD` env var (required for compilation)
- Builds Python/JS guest WASM components and AOT compiles them
- Manages `wasm-tools` and `hyperlight-wasm-aot` tool installation
- Recipes: `guest-build`, `js-guest-build`, `build`, `test`, `examples`, `lint`, `clean`
- `examples` is a flat recipe listing all `cargo run` commands (no per-example sub-recipes)

### js (`src/javascript_sandbox/Justfile`)
- Needs WIT world from wasm for compilation
- Recipes: `build`, `test`, `examples`, `lint`
- `examples` is a flat recipe listing all `cargo run` commands

### nanvix (`src/nanvix_sandbox/Justfile`)
- Standalone build, excluded from workspace
- Recipes: `build`, `examples` (currently skipped)

### python (`src/sdk/python/Justfile`)
- Manages `uv`, `maturin`, `ruff` tooling
- Recipes: `build`, `fmt`, `fmt-check`, `lint`, `examples`, `python-test`, `python-fuzz`, `python-sandbox-benchmark`, `python-publish`
- `examples` is a flat recipe listing all `uv run python` commands
- `lint-rust` in root needs `--features test-utils` because `--all-targets` compiles integration tests

## Key Environment Variables

| Variable | Required by | Purpose |
|----------|------------|---------|
| `WIT_WORLD` | WASM + JS sandbox builds | Path to compiled WIT world (`src/wasm_sandbox/wit/sandbox-world.wasm`) |
| `COPILOT_GITHUB_TOKEN` | Integration examples | GitHub token for Copilot SDK examples |

## CI Job Dependencies and Setup

All jobs need:
- `just` installed via `cargo install --locked just`

WASM/JS jobs additionally need:
- KVM enabled (for Hyperlight VM)
- `clang` installed

WASM job additionally needs:
- Python 3.12 + `uv` (for guest componentize-py)
- Node.js + npm (for JS guest build)
