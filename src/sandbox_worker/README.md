# Developer sandbox worker

Experimental, **developer-only** process execution for the macOS Python and .NET
SDKs. This is not a production isolation service or a public protocol.

The design follows [HIP-0004](https://github.com/hyperlight-dev/hyperlight/pull/1843):
one sandbox per worker, a bounded reusable pool, high-level operations, nested
tool callbacks, versioned length-prefixed `postcard` messages on an inherited
Unix-domain socket, and snapshots transferred through private OCI-layout paths.
There is no listening socket, port, or separate authentication exchange.

The SDK API stays unchanged. On macOS, the native binding starts the packaged,
entitled helper instead of creating a VM in Python or `dotnet`. Only our helper
is signed. A private executable copy handles package installs that do not retain
execute permissions, without modifying installed packages or application hosts.
Linux and Windows SDKs keep their existing in-process defaults. The process path
is also tested on Linux; Windows transport is not implemented in this first step.

Run `just worker build` to build the helper, or `just worker test` for real guest
tests. For a manually built SDK, `HYPERLIGHT_SANDBOX_WORKER_PATH` can specify an absolute
helper path. Normal SDK builds and packages place it beside the native module.

`HYPERLIGHT_SANDBOX_INITIAL_WORKERS` defaults to 0;
`HYPERLIGHT_SANDBOX_MAX_WORKERS` defaults to 16. Exhaustion fails immediately.
These settings are separate from core Hyperlight's Windows surrogate settings.
Snapshots keep their private files alive across restores and source-sandbox
destruction. Worker failure fails pending work; it does not replay guest code.
Frames are limited to 256 MiB and oversized messages fail explicitly.

The Rust builder and protocol are private SDK plumbing. Replace this crate with
upstream support when it covers Wasm/HyperlightJS and the SDK semantics.
