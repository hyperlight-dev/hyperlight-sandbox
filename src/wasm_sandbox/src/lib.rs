extern crate alloc;

use std::backtrace::Backtrace;
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use hyperlight_sandbox::{
    CapFs, ExecutionResult, Guest, GuestSandbox, NetworkPermissions, SandboxConfig, Snapshot,
    ToolRegistry,
};
use hyperlight_wasm::{
    LoadedWasmSandbox, SandboxBuilder as HyperlightSandboxBuilder, Snapshot as WasmSnapshot,
};

mod wasi_impl;

pub(crate) mod bindings {
    hyperlight_component_macro::host_bindgen!("wit/sandbox-world.wasm");
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Wasm;

impl Guest for Wasm {
    type Sandbox = WasmComponentSandbox;
    fn build(
        self,
        config: SandboxConfig,
        tools: ToolRegistry,
        network: std::sync::Arc<std::sync::Mutex<NetworkPermissions>>,
        fs: std::sync::Arc<std::sync::Mutex<CapFs>>,
    ) -> Result<WasmComponentSandbox> {
        WasmComponentSandbox::with_tools(config, tools, network, fs)
    }
}

pub struct HostState {
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) fs: Arc<Mutex<CapFs>>,
    pub(crate) network: Arc<Mutex<NetworkPermissions>>,
    pub(crate) active_requests: Arc<AtomicUsize>,
}

#[allow(refining_impl_trait)]
impl bindings::root::component::RootImports for HostState {
    type Tools = HostState;
    fn tools(&mut self) -> &mut Self {
        self
    }

    type Environment = HostState;
    fn environment(&mut self) -> &mut Self {
        self
    }

    type Exit = HostState;
    fn exit(&mut self) -> &mut Self {
        self
    }

    type Error = HostState;
    fn error(&mut self) -> &mut Self {
        self
    }

    type Poll = HostState;
    fn poll(&mut self) -> &mut Self {
        self
    }

    type Streams = HostState;
    fn streams(&mut self) -> &mut Self {
        self
    }

    type Stdin = HostState;
    fn stdin(&mut self) -> &mut Self {
        self
    }

    type Stdout = HostState;
    fn stdout(&mut self) -> &mut Self {
        self
    }

    type Stderr = HostState;
    fn stderr(&mut self) -> &mut Self {
        self
    }

    type TerminalInput = HostState;
    fn terminal_input(&mut self) -> &mut Self {
        self
    }

    type TerminalOutput = HostState;
    fn terminal_output(&mut self) -> &mut Self {
        self
    }

    type TerminalStdin = HostState;
    fn terminal_stdin(&mut self) -> &mut Self {
        self
    }

    type TerminalStdout = HostState;
    fn terminal_stdout(&mut self) -> &mut Self {
        self
    }

    type TerminalStderr = HostState;
    fn terminal_stderr(&mut self) -> &mut Self {
        self
    }

    type MonotonicClock = HostState;
    fn monotonic_clock(&mut self) -> &mut Self {
        self
    }

    type WallClock = HostState;
    fn wall_clock(&mut self) -> &mut Self {
        self
    }

    type WasiFilesystemTypes = HostState;
    fn wasi_filesystem_types(&mut self) -> &mut Self {
        self
    }

    type WasiHttpTypes = HostState;
    fn wasi_http_types(&mut self) -> &mut Self {
        self
    }

    type OutgoingHandler = HostState;
    fn outgoing_handler(&mut self) -> &mut Self {
        self
    }

    type Preopens = HostState;
    fn preopens(&mut self) -> &mut Self {
        self
    }

    type Network = HostState;
    fn network(&mut self) -> &mut Self {
        self
    }

    type InstanceNetwork = HostState;
    fn instance_network(&mut self) -> &mut Self {
        self
    }

    type Udp = HostState;
    fn udp(&mut self) -> &mut Self {
        self
    }

    type UdpCreateSocket = HostState;
    fn udp_create_socket(&mut self) -> &mut Self {
        self
    }

    type Tcp = HostState;
    fn tcp(&mut self) -> &mut Self {
        self
    }

    type TcpCreateSocket = HostState;
    fn tcp_create_socket(&mut self) -> &mut Self {
        self
    }

    type IpNameLookup = HostState;
    fn ip_name_lookup(&mut self) -> &mut Self {
        self
    }

    type Random = HostState;
    fn random(&mut self) -> &mut Self {
        self
    }

    type Insecure = HostState;
    fn insecure(&mut self) -> &mut Self {
        self
    }

    type InsecureSeed = HostState;
    fn insecure_seed(&mut self) -> &mut Self {
        self
    }
}

impl bindings::hyperlight::sandbox::Tools for HostState {
    fn dispatch(
        &mut self,
        name: String,
        args_json: String,
    ) -> Result<Result<String, String>, hyperlight_host::HyperlightError> {
        let args: serde_json::Value = match serde_json::from_str(&args_json) {
            Ok(args) => args,
            Err(error) => return Ok(Err(error.to_string())),
        };
        Ok(match self.tools.dispatch(&name, args) {
            Ok(v) => match serde_json::to_string(&v) {
                Ok(s) => Ok(s),
                Err(e) => Err(format!("serialization failed: {e}")),
            },
            Err(e) => Err(e.to_string()),
        })
    }
}

pub struct WasmComponentSandbox {
    sandbox: bindings::RootSandbox<HostState, LoadedWasmSandbox>,
    fs: Arc<Mutex<CapFs>>,
}

impl WasmComponentSandbox {
    fn with_tools(
        config: SandboxConfig,
        tools: ToolRegistry,
        network: Arc<Mutex<NetworkPermissions>>,
        fs: Arc<Mutex<CapFs>>,
    ) -> Result<Self> {
        // Verify the shared tokio runtime is available before proceeding.
        hyperlight_sandbox::runtime::RUNTIME
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        if config.module_path.is_empty() {
            anyhow::bail!("module_path is required for the generic Wasm backend");
        }

        let module_path = config.module_path.clone();
        let tools = Arc::new(tools);
        let state = HostState {
            tools: tools.clone(),
            fs: fs.clone(),
            network: network.clone(),
            active_requests: Arc::new(AtomicUsize::new(0)),
        };

        let mut proto = HyperlightSandboxBuilder::new()
            .with_guest_input_buffer_size(config.heap_size.min(70_000_000) as usize)
            .with_guest_heap_size(config.heap_size)
            .with_guest_scratch_size(config.stack_size as usize)
            .build()
            .context("failed to build ProtoWasmSandbox")?;

        let rt = bindings::register_host_functions(&mut proto, state);

        let wasm_sandbox = proto
            .load_runtime()
            .context("failed to load Wasm runtime")?;

        let sb = wasm_sandbox
            .load_module(Path::new(&module_path))
            .context("failed to load Wasm module")?;

        Ok(Self {
            sandbox: bindings::RootSandbox { sb, rt },
            fs,
        })
    }

    fn run_impl(&mut self, code: &str) -> Result<ExecutionResult> {
        use bindings::hyperlight::sandbox::Executor;

        self.fs
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?
            .clear_output_files();

        let code_owned = code.to_string();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.sandbox.run(code_owned)
        }));

        match result {
            Ok(wit_result) => {
                let wit_result =
                    wit_result.map_err(|e| anyhow::anyhow!("guest execution failed: {e}"))?;
                Ok(ExecutionResult {
                    stdout: wit_result.stdout,
                    stderr: wit_result.stderr,
                    exit_code: wit_result.exit_code,
                })
            }
            Err(panic_info) => {
                let backtrace = Backtrace::capture();
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "sandbox execution failed".to_string()
                };
                let stderr = if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
                    format!("{msg}\nBacktrace:\n{backtrace}")
                } else {
                    msg
                };
                Ok(ExecutionResult {
                    stdout: String::new(),
                    stderr,
                    exit_code: -1,
                })
            }
        }
    }
}

impl GuestSandbox for WasmComponentSandbox {
    type SnapshotData = WasmSnapshot;

    fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        self.run_impl(code)
    }

    fn snapshot(&mut self) -> Result<Snapshot<WasmSnapshot>> {
        let runtime = self
            .sandbox
            .sb
            .snapshot()
            .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;
        Ok(Snapshot::new("wasm-component", runtime))
    }

    fn restore(&mut self, snapshot: &Snapshot<WasmSnapshot>) -> Result<()> {
        self.sandbox
            .sb
            .restore(snapshot.snapshot().clone())
            .map_err(|e| anyhow::anyhow!("restore failed: {e}"))
    }
}
