//! High-level host library for running sandbox guests across multiple backends.

extern crate alloc;

pub mod cap_fs;
pub mod http;
pub mod network;
pub mod runtime;
#[cfg(feature = "test-utils")]
pub mod test_utils;
pub mod tools;

use std::any::Any;
use std::path::{Path, PathBuf};

use anyhow::Result;
pub use cap_fs::{
    CapFs, DescriptorFlags, DescriptorStat, DescriptorType, Dir, DirPerms, FilePerms, FsError,
    OpenFlags,
};
pub use network::{HttpMethod, MethodFilter, NetworkPermission, NetworkPermissions};
use serde::{Deserialize, Serialize};
pub use tools::{ArgType, ToolRegistry, ToolSchema};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for building a sandbox guest.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Path to the AOT-compiled Wasm component (e.g. `python-sandbox.aot`).
    pub module_path: String,
    /// Guest heap size in bytes.
    pub heap_size: u64,
    /// Guest scratch / stack size in bytes.
    pub stack_size: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            module_path: String::new(),
            heap_size: 200 * 1024 * 1024,
            stack_size: 100 * 1024 * 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// Execution result
// ---------------------------------------------------------------------------

/// The result of executing code inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Snapshot {
    kind: &'static str,
    runtime: std::sync::Arc<dyn Any + Send + Sync>,
}

impl Snapshot {
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub fn new<T>(kind: &'static str, runtime: std::sync::Arc<T>) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self { kind, runtime }
    }

    pub fn restore<T>(
        &self,
        restore_runtime: impl FnOnce(std::sync::Arc<T>) -> Result<()>,
        fs: &std::sync::Arc<std::sync::Mutex<CapFs>>,
    ) -> Result<()>
    where
        T: Any + Send + Sync + 'static,
    {
        let runtime = self
            .runtime
            .clone()
            .downcast::<T>()
            .map_err(|_| anyhow::anyhow!("snapshot type mismatch (kind: {})", self.kind))?;
        restore_runtime(runtime)?;
        fs.lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned during snapshot restore"))?
            .clear_output_files();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Guest traits
// ---------------------------------------------------------------------------

pub trait Guest: Sized {
    fn build(
        self,
        config: SandboxConfig,
        tools: ToolRegistry,
        network: std::sync::Arc<std::sync::Mutex<NetworkPermissions>>,
        fs: std::sync::Arc<std::sync::Mutex<CapFs>>,
    ) -> Result<Box<dyn GuestSandbox>>;
}

pub trait GuestSandbox: Send {
    /// Execute guest code.
    ///
    /// Output files under `/output` are wiped before each execution.
    /// Input files are read-only and managed by the host.
    fn run(&mut self, code: &str) -> Result<ExecutionResult>;
    /// Capture a snapshot of the guest runtime state.
    fn snapshot(&mut self) -> Result<Snapshot>;
    /// Restore a previously captured guest runtime state.
    fn restore(&mut self, snapshot: &Snapshot) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

pub struct Sandbox {
    inner: Box<dyn GuestSandbox>,
    network: std::sync::Arc<std::sync::Mutex<NetworkPermissions>>,
    fs: std::sync::Arc<std::sync::Mutex<CapFs>>,
}

impl Sandbox {
    /// Create a sandbox without filesystem access.
    pub fn new<G: Guest>(guest: G, config: SandboxConfig, tools: ToolRegistry) -> Result<Self> {
        let network = std::sync::Arc::new(std::sync::Mutex::new(NetworkPermissions::new()));
        let fs = std::sync::Arc::new(std::sync::Mutex::new(CapFs::new()));
        let inner = guest.build(config, tools, network.clone(), fs.clone())?;
        Ok(Self { inner, network, fs })
    }

    /// Create a sandbox with a read-only input directory.
    pub fn with_input<G: Guest>(
        guest: G,
        config: SandboxConfig,
        tools: ToolRegistry,
        input_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let network = std::sync::Arc::new(std::sync::Mutex::new(NetworkPermissions::new()));
        let fs = CapFs::new().with_input(input_dir)?;
        let fs = std::sync::Arc::new(std::sync::Mutex::new(fs));
        let inner = guest.build(config, tools, network.clone(), fs.clone())?;
        Ok(Self { inner, network, fs })
    }

    pub fn builder() -> SandboxBuilder<NoGuest> {
        SandboxBuilder::default()
    }

    /// Execute guest code.
    ///
    /// Output files under `/output` are cleared before each run. Input files
    /// persist until `clear_files` is called.
    pub fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        self.inner.run(code)
    }

    pub fn snapshot(&mut self) -> Result<Snapshot> {
        self.inner.snapshot()
    }

    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<()> {
        self.inner.restore(snapshot)
    }

    /// List filenames in the output directory (without reading contents).
    pub fn get_output_files(&self) -> Result<Vec<String>> {
        Ok(self
            .fs
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?
            .get_output_files())
    }

    /// Return the host filesystem path of the output directory, if configured.
    pub fn output_path(&self) -> Result<Option<std::path::PathBuf>> {
        Ok(self
            .fs
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?
            .output_path()
            .map(|p| p.to_path_buf()))
    }

    pub fn allow_domain(&mut self, target: &str, methods: impl Into<MethodFilter>) -> Result<()> {
        self.network
            .lock()
            .map_err(|_| anyhow::anyhow!("network mutex poisoned"))?
            .allow_domain(target, methods)
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Typestate marker indicating no guest backend has been selected yet.
/// Prevents calling `.build()` before `.guest(...)`.
pub struct NoGuest;

/// Builder for constructing a [`Sandbox`].
///
/// ```rust,ignore
/// let sandbox = Sandbox::builder()
///     .module_path("guest.aot")
///     .output_dir("/tmp/sandbox-out")
///     .guest(Wasm)
///     .build()?;
/// ```
pub struct SandboxBuilder<G = NoGuest> {
    guest: G,
    config: SandboxConfig,
    tools: ToolRegistry,
    input_dir: Option<PathBuf>,
    output_dir: Option<(PathBuf, DirPerms, FilePerms)>,
    temp_output: bool,
}

impl Default for SandboxBuilder<NoGuest> {
    fn default() -> Self {
        Self {
            guest: NoGuest,
            config: SandboxConfig::default(),
            tools: ToolRegistry::default(),
            input_dir: None,
            output_dir: None,
            temp_output: false,
        }
    }
}

impl<G> SandboxBuilder<G> {
    pub fn module_path(mut self, module_path: impl Into<String>) -> Self {
        self.config.module_path = module_path.into();
        self
    }

    pub fn heap_size(mut self, heap_size: u64) -> Self {
        self.config.heap_size = heap_size;
        self
    }

    pub fn stack_size(mut self, stack_size: u64) -> Self {
        self.config.stack_size = stack_size;
        self
    }

    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn tool_typed<T, F>(mut self, name: &str, handler: F) -> Self
    where
        T: serde::de::DeserializeOwned + Send + 'static,
        F: Fn(T) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        self.tools.register_typed::<T, F>(name, handler);
        self
    }

    /// Set the host directory exposed as the read-only `/input` preopen.
    pub fn input_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.input_dir = Some(path.into());
        self
    }

    /// Set the host directory exposed as the writable `/output` preopen,
    /// with explicit permissions. Without this, output uses a temp directory
    /// with full read-write access.
    pub fn output_dir(
        mut self,
        path: impl Into<PathBuf>,
        dir_perms: DirPerms,
        file_perms: FilePerms,
    ) -> Self {
        self.output_dir = Some((path.into(), dir_perms, file_perms));
        self
    }

    /// Enable a temporary writable `/output` directory. Ignored when an
    /// explicit `output_dir` is set.
    pub fn temp_output(mut self) -> Self {
        self.temp_output = true;
        self
    }
}

impl SandboxBuilder<NoGuest> {
    pub fn guest<G>(self, guest: G) -> SandboxBuilder<G>
    where
        G: Guest,
    {
        SandboxBuilder {
            guest,
            config: self.config,
            tools: self.tools,
            input_dir: self.input_dir,
            output_dir: self.output_dir,
            temp_output: self.temp_output,
        }
    }
}

impl<G> SandboxBuilder<G>
where
    G: Guest,
{
    pub fn build(self) -> Result<Sandbox> {
        let network = std::sync::Arc::new(std::sync::Mutex::new(NetworkPermissions::new()));
        let mut vfs = CapFs::new();
        if let Some(input_dir) = &self.input_dir {
            vfs = vfs.with_input(input_dir)?;
        }
        vfs = match self.output_dir {
            Some((path, dir_perms, file_perms)) => {
                vfs.with_output_dir(path, dir_perms, file_perms)?
            }
            None if self.temp_output => vfs.with_temp_output()?,
            None => vfs,
        };
        let fs = std::sync::Arc::new(std::sync::Mutex::new(vfs));
        let inner = self
            .guest
            .build(self.config, self.tools, network.clone(), fs.clone())?;
        Ok(Sandbox { inner, network, fs })
    }
}
