# Copilot Instructions

- When running examples in this repository, use the `Justfile` recipes instead of invoking `cargo run` or `python` directly.
- Use `just examples` from the repository root to run the full example suite.
- To run examples for a specific sandbox, use module-scoped recipes: `just wasm examples`, `just js examples`, `just python examples`.
- Use `just build-all` from the repository root to build all subprojects and SDKs.
- Reason: the example commands depend on `WIT_WORLD` being set to `src/wasm_sandbox/wit/sandbox-world.wasm`; the `Justfile` handles that setup.

Make things cross-platform where possible (window/mac/linux).  Mac supprot for hyperlight isn't avaliable yet but is coming.

- **After changing WIT interfaces**: you must run `just build-all` (or at minimum rebuild the guest `.wasm` and `.aot` files) before running examples. The pre-compiled guest binaries embed the WIT signature; a mismatch causes "Host function vector parameter missing length" errors at runtime.

- **Formatting and linting**: always use `just fmt` and `just fmt-check` from the repository root instead of invoking `cargo fmt`, `ruff format`, or `ruff check` directly. The Justfile recipes run multiple tools in sequence (e.g. `ruff format` + `ruff check --fix` for Python) and missing a step causes CI failures.