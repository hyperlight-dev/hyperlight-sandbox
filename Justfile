set unstable := true

mod wasm 'src/wasm_sandbox/Justfile'
mod js 'src/javascript_sandbox/Justfile'
mod nanvix 'src/nanvix_sandbox/Justfile'
mod python 'src/sdk/python/Justfile'
mod examples_mod 'examples/Justfile'

default-target := "debug"

clean: wasm::clean python::clean
    cargo clean

#### BUILD TARGETS ####

build target=default-target: (wasm::build target) (js::build target) nanvix::build python::build

lint: lint-rust wasm::lint js::lint python::lint

lint-rust:
    cargo clippy -p hyperlight-sandbox --all-targets --features test-utils -- -D warnings

fmt: fmt-rust python::fmt

fmt-rust:
    cargo +nightly fmt --all

fmt-check: fmt-check-rust python::fmt-check

fmt-check-rust:
    cargo +nightly fmt --all -- --check

#### TESTS ####

test: wasm::guest-build wasm::js-guest-build python::build python::python-test test-rust wasm::test

fuzz seconds="60": (python::python-fuzz seconds)

test-rust:
    cargo test -p hyperlight-sandbox --features test-utils

benchmark: python::python-sandbox-benchmark

python-dist: (wasm::build "release") (js::build "release") python::python-dist

python-dist-backends: wasm::_clean-stale-wasm wasm::guest-compile-wit js::_clean-stale
    cargo build --manifest-path src/wasm_sandbox/Cargo.toml --release
    cargo build --manifest-path src/javascript_sandbox/Cargo.toml --release
    just python python-dist-backends

python-wheelhouse-test: python-dist python::python-wheelhouse-test

examples target=default-target: (wasm::examples target) (js::examples target) python::examples

integration-examples target=default-target: (wasm::guest-build target) wasm::js-guest-build python::build examples_mod::integration-examples


#### DOCS ####

slides-build:
    npx --yes @marp-team/marp-cli docs/end-user-overview-slides.md -o docs/end-user-overview-slides.html

slides:
    npx --yes @marp-team/marp-cli --server --watch docs/

##### Run GitHub Actions CI locally using act (https://nektosact.com) #######

[unix]
ci job="":
    #!/usr/bin/env bash
    if ! command -v act &>/dev/null; then
        echo "act is not installed. Install it from: https://nektosact.com/installation/index.html"
        exit 1
    fi
    args=(-e {{ replace(justfile_directory(), "\\", "/") }}/.github/act-event.json)
    if [ -e /dev/kvm ]; then
        args+=(--container-options "--device /dev/kvm")
    fi
    if command -v gh &>/dev/null; then
        args+=(-s "COPILOT_TOKEN=$(gh auth token)")
    fi
    if [ -z "{{ job }}" ]; then
        act "${args[@]}"
    else
        act -j "{{ job }}" "${args[@]}"
    fi
