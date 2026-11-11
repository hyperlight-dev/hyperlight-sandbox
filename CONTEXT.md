# Hyperlight Sandbox

Hyperlight Sandbox lets applications run guest code with explicit access to
application tools, files, and networks.

## Language

**Application**:
The program that uses a sandbox through an SDK. It is distinct from the guest
program.

**Guest**:
The program that runs inside a sandbox.

**Sandbox**:
An isolated guest execution instance with its own configuration and state.

**Guest backend**:
The runtime used to execute guest code, such as Wasm or HyperlightJS.
_Avoid_: Using backend to mean a process or communication channel.

**Sandbox host**:
The component that owns and operates a sandbox. It can be part of the application
or separate from it.

**Host tool**:
A named application function that guest code can call.

**Snapshot**:
A checkpoint of guest execution state. It is not a backup of the application's
files or external services.
