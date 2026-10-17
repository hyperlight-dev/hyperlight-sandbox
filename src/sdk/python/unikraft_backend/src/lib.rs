use std::collections::HashMap;

use hyperlight_sandbox::{
    DirPerms, FilePerms, FilesystemLimits, HttpMethod, MethodFilter, Sandbox, SandboxBuilder,
};
use hyperlight_sandbox_pyo3_common::{
    PyExecutionResult, build_tool_registry, parse_filesystem_limits, parse_size,
    parse_tool_registration,
};
use hyperlight_unikraft_sandbox::{Unikraft, cached_rootfs};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

type UnikraftSandboxInner = Sandbox<Unikraft>;
type UnikraftSnapshotInner = hyperlight_sandbox::Snapshot<<
    <Unikraft as hyperlight_sandbox::Guest>::Sandbox as hyperlight_sandbox::GuestSandbox
>::SnapshotData>;

#[pyclass]
pub struct PySnapshot {
    inner: UnikraftSnapshotInner,
}

#[pyclass(unsendable)]
pub struct UnikraftSandbox {
    inner: Option<UnikraftSandboxInner>,
    tools: HashMap<String, Py<PyAny>>,
    pending_networks: Vec<(String, MethodFilter)>,
    rootfs: String,
    guest: Unikraft,
    input_dir: Option<String>,
    output_dir: Option<String>,
    temp_output: bool,
    filesystem_limits: Option<FilesystemLimits>,
}

#[pymethods]
impl UnikraftSandbox {
    /// `module_path` is a rootfs image (initrd CPIO); without one, the
    /// published image named by `runtime` (default `agent`) is taken from
    /// the `hluk` CLI's cache. `scratch_size` overrides the guest memory
    /// the image was tested with (e.g. `"2Gi"`). `native_network` also lets
    /// guest sockets reach the allowed domains (by host name only).
    #[new]
    #[pyo3(signature = (input_dir=None, output_dir=None, temp_output=false, module_path=None, runtime=None, scratch_size=None, native_network=false, filesystem_limits=None, max_file_size=None, max_total_size=None, max_file_count=None))]
    fn new(
        input_dir: Option<&str>,
        output_dir: Option<&str>,
        temp_output: bool,
        module_path: Option<&str>,
        runtime: Option<&str>,
        scratch_size: Option<&str>,
        native_network: bool,
        filesystem_limits: Option<&str>,
        max_file_size: Option<&str>,
        max_total_size: Option<&str>,
        max_file_count: Option<usize>,
    ) -> PyResult<Self> {
        let rootfs = match (module_path, runtime) {
            (Some(_), Some(_)) => {
                return Err(PyValueError::new_err(
                    "pass either module_path or runtime, not both",
                ));
            }
            (Some(path), None) => path.to_string(),
            (None, runtime) => cached_rootfs(runtime.unwrap_or("agent"))
                .map_err(|e| PyRuntimeError::new_err(format!("{e:#}")))?
                .display()
                .to_string(),
        };
        let mut guest = Unikraft::new().native_network(native_network);
        if let Some(size) = scratch_size {
            let mb = usize::try_from(parse_size(size)? / (1024 * 1024))
                .map_err(|_| PyValueError::new_err("scratch_size is too large"))?;
            if mb == 0 {
                return Err(PyValueError::new_err("scratch_size must be at least 1Mi"));
            }
            guest = guest.scratch_mb(mb);
        }
        Ok(UnikraftSandbox {
            inner: None,
            tools: HashMap::new(),
            pending_networks: Vec::new(),
            rootfs,
            guest,
            input_dir: input_dir.map(|s| s.to_string()),
            output_dir: output_dir.map(|s| s.to_string()),
            temp_output,
            filesystem_limits: parse_filesystem_limits(
                filesystem_limits,
                max_file_size,
                max_total_size,
                max_file_count,
            )?,
        })
    }

    #[pyo3(signature = (name_or_tool, callback=None))]
    fn register_tool(
        &mut self,
        py: Python<'_>,
        name_or_tool: Py<PyAny>,
        callback: Option<Py<PyAny>>,
    ) -> PyResult<()> {
        if self.inner.is_some() {
            return Err(PyRuntimeError::new_err(
                "Cannot register tools after sandbox has been initialized. \
                 Register all tools before the first run() call.",
            ));
        }
        let (name, cb) = parse_tool_registration(py, name_or_tool, callback)?;
        self.tools.insert(name, cb);
        Ok(())
    }

    #[pyo3(signature = (code))]
    fn run(&mut self, py: Python<'_>, code: &str) -> PyResult<PyExecutionResult> {
        if self.inner.is_none() {
            let registry = build_tool_registry(py, &mut self.tools)?;
            let mut builder = SandboxBuilder::new()
                .module_path(&self.rootfs)
                .with_tools(registry)
                .guest(self.guest);
            if let Some(limits) = self.filesystem_limits {
                builder = builder.filesystem_limits(limits);
            }
            if let Some(ref dir) = self.input_dir {
                builder = builder.input_dir(dir);
            }
            if let Some(ref dir) = self.output_dir {
                builder = builder.output_dir(
                    dir,
                    DirPerms::READ | DirPerms::MUTATE,
                    FilePerms::READ | FilePerms::WRITE,
                );
            } else if self.temp_output {
                builder = builder.temp_output();
            }
            // Booting the micro-VM takes no Python state: let other
            // Python threads run meanwhile.
            let mut sandbox = py
                .detach(|| builder.build())
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create sandbox: {e:#}")))?;
            for (target, methods) in std::mem::take(&mut self.pending_networks) {
                sandbox
                    .allow_domain(&target, methods)
                    .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
            }
            self.inner = Some(sandbox);
        }
        let sandbox = self.inner.as_mut().unwrap();
        let result = sandbox
            .run(code)
            .map_err(|e| PyRuntimeError::new_err(format!("Execution failed: {e}")))?;
        Ok(PyExecutionResult {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        })
    }

    #[pyo3(signature = (target, methods=None))]
    fn allow_domain(&mut self, target: &str, methods: Option<Vec<String>>) -> PyResult<()> {
        let methods =
            HttpMethod::parse_list(methods).map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
        if let Some(sandbox) = self.inner.as_mut() {
            sandbox
                .allow_domain(target, methods)
                .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
        } else {
            self.pending_networks.push((target.to_string(), methods));
        }
        Ok(())
    }

    fn snapshot(&mut self) -> PyResult<PySnapshot> {
        let sandbox = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("Sandbox not initialized"))?;
        let snap = sandbox
            .snapshot()
            .map_err(|e| PyRuntimeError::new_err(format!("Snapshot failed: {e}")))?;
        Ok(PySnapshot { inner: snap })
    }

    fn restore(&mut self, snapshot: &PySnapshot) -> PyResult<()> {
        let sandbox = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("Sandbox not initialized"))?;
        sandbox
            .restore(&snapshot.inner)
            .map_err(|e| PyRuntimeError::new_err(format!("Restore failed: {e}")))?;
        Ok(())
    }

    fn get_output_files(&self) -> PyResult<Vec<String>> {
        let sandbox = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Sandbox not initialized"))?;
        sandbox
            .get_output_files()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to get output files: {e}")))
    }

    fn output_path(&self) -> PyResult<Option<String>> {
        let sandbox = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Sandbox not initialized"))?;
        let path = sandbox
            .output_path()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to get output path: {e}")))?;
        Ok(path.map(|p| p.display().to_string()))
    }
}

#[pymodule]
fn _native_unikraft(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<UnikraftSandbox>()?;
    m.add_class::<PyExecutionResult>()?;
    m.add_class::<PySnapshot>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
