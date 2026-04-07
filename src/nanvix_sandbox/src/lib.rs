//! Nanvix microkernel sandbox backend for hyperlight-sandbox.
//!
//! Runs JavaScript or Python workloads inside a Nanvix microkernel VM.
//!
//! **Limitations** compared to the Wasm/JS sandbox backends:
//! - No host function / tool dispatch (guest code cannot call `call_tool`)
//! - No snapshot/restore
//! - File I/O and network are not available through the sandbox API
//! - Output is captured from a guest console log file, not from WASI streams
//!
//! Supported:
//! - Basic code execution (JS via QuickJS, Python via CPython)
//! - stdout capture from the guest console log

use std::io::Write;

use anyhow::{Context, Result};
use hyperlight_nanvix::{RuntimeConfig, Sandbox as NanvixSandboxInner, WorkloadType};
use hyperlight_sandbox::{
    ExecutionResult, Guest, GuestSandbox, SandboxConfig, Snapshot, ToolRegistry,
};

// ---------------------------------------------------------------------------
// Guest types
// ---------------------------------------------------------------------------

/// Guest type for running JavaScript via QuickJS in a Nanvix microkernel.
#[derive(Debug, Clone, Copy, Default)]
pub struct NanvixJavaScript;

impl Guest for NanvixJavaScript {
    type Sandbox = NanvixSandbox;
    fn build(
        self,
        _config: SandboxConfig,
        _tools: ToolRegistry,
        _network: std::sync::Arc<std::sync::Mutex<hyperlight_sandbox::NetworkPermissions>>,
        _fs: std::sync::Arc<std::sync::Mutex<hyperlight_sandbox::CapFs>>,
    ) -> Result<NanvixSandbox> {
        NanvixSandbox::new(WorkloadType::JavaScript)
    }
}

/// Guest type for running Python via CPython in a Nanvix microkernel.
#[derive(Debug, Clone, Copy, Default)]
pub struct NanvixPython;

impl Guest for NanvixPython {
    type Sandbox = NanvixSandbox;
    fn build(
        self,
        _config: SandboxConfig,
        _tools: ToolRegistry,
        _network: std::sync::Arc<std::sync::Mutex<hyperlight_sandbox::NetworkPermissions>>,
        _fs: std::sync::Arc<std::sync::Mutex<hyperlight_sandbox::CapFs>>,
    ) -> Result<NanvixSandbox> {
        NanvixSandbox::new(WorkloadType::Python)
    }
}

// ---------------------------------------------------------------------------
// Sandbox implementation
// ---------------------------------------------------------------------------

pub struct NanvixSandbox {
    workload_type: WorkloadType,
    runtime_config: RuntimeConfig,
    /// Tokio runtime for async nanvix API
    rt: tokio::runtime::Runtime,
}

impl NanvixSandbox {
    fn new(workload_type: WorkloadType) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to create tokio runtime")?;

        Ok(Self {
            workload_type,
            runtime_config: RuntimeConfig::default(),
            rt,
        })
    }

    fn file_extension(&self) -> &'static str {
        match self.workload_type {
            WorkloadType::JavaScript => "js",
            WorkloadType::Python => "py",
            WorkloadType::Binary => "elf",
        }
    }
}

/// Unit type used as snapshot data for backends that don't support snapshotting.
pub struct NoSnapshot;

impl GuestSandbox for NanvixSandbox {
    type SnapshotData = NoSnapshot;

    fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        // Write code to a temporary file
        let mut tmp = tempfile::Builder::new()
            .suffix(&format!(".{}", self.file_extension()))
            .tempfile()
            .context("failed to create temp file for guest code")?;
        tmp.write_all(code.as_bytes())
            .context("failed to write guest code")?;
        tmp.flush()?;

        let tmp_path = tmp.path().to_path_buf();
        let config = self.runtime_config.clone();
        let console_log_path = format!("{}/guest-console.log", &config.log_directory);

        // Run the workload
        let run_result = self.rt.block_on(async {
            let mut sandbox = NanvixSandboxInner::new(config)?;
            sandbox.run(&tmp_path).await
        });

        // Read captured console output
        let stdout = std::fs::read_to_string(&console_log_path).unwrap_or_default();

        // Clean up log directory
        let _ = std::fs::remove_dir_all(&self.runtime_config.log_directory);
        // Rotate to a fresh config for the next run
        self.runtime_config = RuntimeConfig::default();

        match run_result {
            Ok(()) => Ok(ExecutionResult {
                stdout,
                stderr: String::new(),
                exit_code: 0,
            }),
            Err(e) => Ok(ExecutionResult {
                stdout,
                stderr: format!("{e:#}"),
                exit_code: 1,
            }),
        }
    }

    fn snapshot(&mut self) -> Result<Snapshot<NoSnapshot>> {
        Err(anyhow::anyhow!(
            "snapshot is not supported by the Nanvix sandbox"
        ))
    }

    fn restore(&mut self, _snapshot: &Snapshot<NoSnapshot>) -> Result<()> {
        Err(anyhow::anyhow!(
            "restore is not supported by the Nanvix sandbox"
        ))
    }
}
