# Release Process

## 1. Update Package Versions

Bump the version in **all** manifest files. For example, to go from `0.1.0` → `0.2.0`:

### Rust (Cargo)

- `Cargo.toml` — `[workspace.package] version` and the versions for local crates in
  `[workspace.dependencies]`

All other workspace member crates inherit the version automatically.

### Python (pyproject.toml)

- `pyproject.toml` (root dev package)
- `src/sdk/python/core/pyproject.toml` (also update `optional-dependencies` version constraints)
- `src/sdk/python/hyperlight_js_backend/pyproject.toml`
- `src/sdk/python/wasm_backend/pyproject.toml`
- `src/sdk/python/wasm_guests/javascript_guest/pyproject.toml`
- `src/sdk/python/wasm_guests/python_guest/pyproject.toml`

### JavaScript (package.json)

- `src/wasm_sandbox/guests/javascript/package.json`

### .NET (Directory.Build.props)

- `src/sdk/dotnet/Directory.Build.props`

## 2. Verify the Build

```sh
just build
just fmt-check
```

## 3. Commit and Merge

Open a PR with the version bump, get it reviewed, and merge to `main`.

## 4. Tag and Publish

```sh
git checkout main
git pull --ff-only
git tag -s -a v0.2.0 -m "v0.2.0"
git push --tags
```

Replace `v0.2.0` with the version you are releasing.

The publish workflow releases the public Rust crates to crates.io in dependency order,
followed independently by the Python and .NET packages.
