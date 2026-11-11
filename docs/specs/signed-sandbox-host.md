# Signed sandbox host for Python and .NET

Status: Draft. This specifies a direction and the checks needed before
implementation. It does not select a final wire format or process model.

## Goal

Let Python and .NET applications use Hyperlight on macOS without re-signing their
Python interpreter, installed `dotnet`, or application executable.

Build the missing process support locally, behind a private Rust interface.
Replace and delete that support when a suitable upstream implementation exists.
Do not make our temporary wire protocol a public SDK contract.

## Why this is needed

Apple requires the executable that uses Hypervisor.framework to carry
`com.apple.security.hypervisor`. Signing a loaded library does not grant that
entitlement to its process.

| How an application runs | Executable that needs the entitlement today |
| --- | --- |
| Rust application | The application's executable |
| Python script | The Python interpreter |
| `dotnet application.dll` | The installed `dotnet` executable |
| .NET apphost or NativeAOT application | The application's executable |

Rust and .NET applications with their own executable can sign that executable.
That remains a valid in-process option. NuGet itself does not require a separate
process. The separate host provides an installation experience that also works
with existing, unmodified application hosts.

The package deployment target is macOS 13 on Apple Silicon. Hyperlight 0.18
calls `hv_vm_config_create()`, which [requires macOS 13 or newer][vm-config].
This is a build target, not a claim that execution on that OS has been verified.

## Proposed boundary

```text
Application process                         Separate process
Python or .NET
      |
Existing SDK API
      |
Native Rust binding
      |
Private runtime interface
      +-- Local in-process adapter
      |
      +-- Local process adapter ---- IPC ---> Signed sandbox host
      |                                            |
      +-- Future upstream adapter                  Hyperlight
                                                   |
                                                   Guest
```

Only the separate host needs the hypervisor entitlement on the process path.
The first proof must verify this with unmodified Python and `dotnet` hosts.

Keep the interface above a complete configured sandbox, not merely above a
hypervisor call. It must cover guest backend selection, module loading, tools,
network permissions, filesystem configuration, execution, snapshots, and
interruption. The existing `GuestSandbox` trait alone does not cover this:
network and filesystem state also live in the enclosing `Sandbox`.

The Python API and .NET API keep their existing method signatures. Their native
bindings can change internally, but must not expose wire messages, process IDs,
or upstream-specific types. All IPC and process management stays in Rust.
Guest backend selection remains Wasm or HyperlightJS; it does not select a
process model.

## Upstream alignment and deletion plan

[hyperlight-dev/hyperlight#1843][hip] proposes `hyperlight-remote`: a Rust wrapper,
signed surrogate processes, high-level operations, callbacks, and process
management. It is a proposal, explicitly experimental and non-production.
Integration with `hyperlight-wasm` and `hyperlight-js` is outside its initial
scope.

Use its high-level boundary as a reference, not its unfinished types as a stable
dependency contract. Keep these local responsibilities in one replaceable Rust
component:

- Process creation, shutdown, failure detection, and any pooling.
- Private protocol types, framing, version checks, and callback dispatch.
- Host executable discovery, extraction, and signature checks.
- Remote sandbox and snapshot ownership.

Keep language-specific argument validation, callback conversion, and public
exception mapping in the SDK bindings.

When upstream is suitable, add an adapter that passes the same conformance
tests. Then remove the replaced protocol, process manager, and helper code.
Some packaging integration may remain. Do not retain two equivalent process
implementations without a specific compatibility requirement.

This is a replacement during an SDK upgrade, not migration of live processes or
snapshot handles. Upstream must first support our guest backends and required
semantics; a dependency-only replacement is not promised.

## Required behavior

| Area | Requirement |
| --- | --- |
| Execution | Preserve guest state between calls and return the existing stdout, stderr, and exit-code result. |
| Initialization | Preserve lazy initialization and the existing point at which tool registration closes. |
| Tools | Keep application callbacks in the application process. Send arguments and results, never closures or function pointers. Preserve synchronous and asynchronous callback behavior. |
| Cancellation | Process interrupt requests while guest execution is busy. Preserve existing cancellation results, late-result handling, and the requirement to restore after interruption. Do not replace normal interruption with killing the host. |
| Snapshots | Preserve reuse, backend compatibility, and supported restores between sandbox instances. Preserve output-file clearing on restore. Do not serialize callbacks or silently replace memory snapshots with disk snapshots. |
| Files and network | Preserve configured directories, quotas, output paths, and domain/method permissions. The host must not gain additional guest permissions. |
| Lifetime | Keep snapshot and interrupt handles safe when they outlive a sandbox. Release their resources when their last owner is gone. |
| Errors | Preserve language-level guest error and cancellation mappings. Distinguish process/protocol failure from guest failure; never return a success-shaped fallback. |
| Concurrency | Preserve existing SDK threading rules. Reject unsupported reentry rather than deadlocking. Callback handling and interruption must not block behind a running guest's execution lock. |

Existing differences between Python and .NET validation are not standardized as
part of this change. Moving an operation must not silently introduce smaller
input, output, or callback-result limits.

After an unexpected host exit, fail outstanding operations and invalidate its
handles. Do not automatically replay guest code or callbacks: they may have
external side effects. New sandboxes can start a new host.

## Private transport

Use a private, inherited local connection, not a listening network service or a
discoverable socket that accepts unrelated clients.

The HIP sketches length-prefixed binary messages with a protocol version and
`postcard` serialization over a Unix-domain socket. Evaluate that shape in the
proof. No JSON-RPC requirement is adopted here. The codec and platform transport
are internal choices; clients and the packaged host upgrade together.

Require a startup version/backend handshake, bounded framing, request
correlation, callback replies, and an independent path for interruption.
Validate message lengths before allocation. Set limits from the SDK's supported
payloads, not arbitrary small defaults. Diagnostics must not corrupt protocol
messages.

A guest execution request can produce nested tool requests before its final
response. Test that flow, including cancellation during a callback. Keep guest
memory and snapshots out of ordinary execution messages.

## Packaging and signing

Python wheels and NuGet packages must locate the appropriate packaged host
without requiring a separate user install, a Rust compiler, or a new launch
command. Produce the host from the same source/version for both SDKs.

Build and sign the macOS host before packaging where possible. If extraction or
local signing is necessary, it must affect only our host in a user-owned,
versioned cache. Never modify Python, `dotnet`, the user's application, or their
existing signatures and entitlements. Do not write beside those executables.

Use explicit packaged paths, not an unrestricted `PATH` search. Preserve
executable permissions and verify host version, architecture, and integrity.
Validate signature behavior after actual wheel and NuGet installation, including
read-only install locations. Do not require users to disable macOS security
checks. Missing or blocked hosts must produce actionable errors, not an
in-process fallback that again requires signing the application.

The implementation must be portable. Whether Linux and Windows use this mode
by default is a separate rollout decision; their existing defaults do not change
without that decision.

## Delivery gates

1. **Prove the entitlement boundary.** From ordinary Python and
   `dotnet application.dll`, execute a guest using an entitled child while the
   parent remains unchanged. Record the OS, host signatures, and package paths.
2. **Prove API compatibility.** Cover both guest backends, tool callbacks,
   repeated calls, snapshots/restores, filesystem/network permissions, and
   interruption. Include the Agent Framework tool path.
3. **Choose the process model.** Compare one host per sandbox with a shared
   host. Check snapshot sharing, callback ownership, cleanup, concurrency,
   memory use, and current HVF limits before choosing. Do not copy the HIP's
   one-sandbox-per-surrogate policy without checking our API.
4. **Measure the cost.** Compare startup, warm execution, callback-heavy calls,
   snapshot/restore, memory, and cleanup against in-process execution on the
   same hardware. Agree acceptable costs before making the process path the
   default; the HIP's latency estimates are not measurements for these SDKs.
5. **Prove package installation.** Exercise installed wheels and NuGet packages
   with unmodified hosts. Test parent exit, child crash, stale handles,
   malformed/oversized frames, version mismatch, and missing entitlements.
6. **Enable the approved rollout.** Remove interpreter/runtime re-signing from
   the default SDK instructions only after the replacement path works.
7. **Replace local code upstream.** Confirm upstream's signing requirements,
   supported guest backends, stability, and cancellation/snapshot semantics.
   Run the same tests through its adapter, then delete superseded local code.

## Decisions still needed

- Process ownership and snapshot sharing, informed by gate 3.
- Initial rollout platforms and acceptable performance costs.
- Final codec, framing limits, and platform-specific IPC implementation.
- Production distribution/signing requirements beyond CI's ad-hoc signing.
- Which upstream API and support guarantees are sufficient for replacement.

No helper implementation, protocol, rollout change, or upstream dependency is
introduced by this document.

[hip]: https://github.com/hyperlight-dev/hyperlight/pull/1843
[vm-config]: https://developer.apple.com/documentation/hypervisor/hv_vm_config_create()
