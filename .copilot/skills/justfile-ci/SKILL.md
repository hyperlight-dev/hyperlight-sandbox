---
name: justfile-ci
description: >
  Guide for editing Justfiles and GitHub Actions CI workflows in this repository.
  Use when the user asks to add, modify, or remove build/test/lint/format steps,
  add new CI jobs, update Justfile recipes, or wire new tasks into the build system.
  Trigger phrases: "add to CI", "add a just recipe", "update the Justfile",
  "add a test step", "wire into CI", "add to the build", "new CI job".
---

# Justfile & CI Editing Guide

## Architecture

This repo uses a **hierarchical Justfile** structure with a matching **GitHub Actions CI** workflow.

### Justfile Hierarchy

```
Justfile (root)                    ← orchestrates everything
├── mod wasm 'src/wasm_sandbox/Justfile'
├── mod jssandbox 'src/javascript_sandbox/Justfile'
├── mod nanvix 'src/nanvix_sandbox/Justfile'
├── mod python 'src/sdk/python/Justfile'
└── mod examples_mod 'examples/Justfile'
```

- Root recipes (`test`, `build-all`, `lint`, `fmt`) delegate to subproject recipes via `mod::recipe` syntax
- Subproject Justfiles own environment setup (e.g. `WIT_WORLD` for WASM)
- Root Justfile uses `set unstable := true` to enable module imports
- Justfiles are organized with `#### SECTION ####` headers (BUILD TARGETS, TESTS, DOCS, etc.)
- Section headers must have a blank line after them to avoid becoming recipe doc comments in `just --list`
- Example recipes are flat: one `examples` recipe listing all `cargo run`/`uv run` commands directly (no per-example helper recipes)

### CI Structure

`.github/workflows/ci.yml` has separate jobs per subproject. Each job calls `just` recipes — never raw `cargo`/`python` commands.

See `references/architecture.md` for the full CI job layout and Justfile recipe map.

## Workflow: Adding a New Step

1. **Add the recipe to the subproject Justfile** — include any env setup, deps, and the actual command
2. **Wire it into the root Justfile** if it should be part of `test`, `lint`, `fmt`, `build-all`, or `examples`
3. **Add the CI step** — call the `just` recipe from the appropriate CI job
4. **Verify alignment** — root `just test` should run the same test steps that CI runs

## Rules

### Justfile Rules
- Always add recipes to the **subproject Justfile first**, then reference from root via `mod::recipe`
- Don't create root-level wrapper recipes for things that can be called as `mod::recipe` directly
- Subproject recipes handle their own env setup (e.g. `WIT_WORLD`, `CARGO_MANIFEST_DIR`)
- Use recipe dependencies for ordering: `test: build lint` not separate commands
- Use `#### SECTION ####` headers to group recipes; keep a blank line after the header
- Keep comments minimal — don't restate the recipe name; only add comments for non-obvious behavior
- Use `default-target` parameter pattern for build profile selection
- Example recipes should be flat lists of commands in one `examples` recipe, not separate per-example recipes

### CI Rules
- CI steps must call `just` recipes — never bypass with raw commands
- Each subproject has its own CI job (don't mix unrelated subprojects)
- Exception: Python SDK runs in the `wasm-sandbox` job to avoid rebuilding
- KVM setup is required for jobs that run Hyperlight sandboxes
- `just` is installed via `cargo install --locked just` in each job

### Alignment Rule
- Root `just test` and CI must test the same things
- When adding a test step to CI, verify it's also called from `just test`
- When adding a `just test` dependency, verify CI runs it too

## Common Patterns

### Adding a test recipe to a subproject
```just
# In subproject Justfile:
test target=default-target:
    {{ wit-world }} cargo test --manifest-path {{repo-root}}/path/Cargo.toml --profile={{ if target == "debug" {"dev"} else { target } }} --test my_test
```

### Wiring it into root
```just
# In root Justfile — add to test deps:
test: wasm::guest-build wasm::js-guest-build python::build python::python-test test-rust wasm::test
```

### Adding to CI
```yaml
# In ci.yml — add step to appropriate job:
      - name: Integration tests
        run: just wasm test
```

### Recipe with feature flags
```just
test-rust:
    cargo test -p hyperlight-sandbox --features test-utils

lint-rust:
    cargo clippy -p hyperlight-sandbox --all-targets --features test-utils -- -D warnings
```
