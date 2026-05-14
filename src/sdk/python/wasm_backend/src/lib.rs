use std::collections::HashMap;
use std::sync::Arc;

use hyperlight_sandbox::{
    DEFAULT_HEAP_SIZE, DEFAULT_STACK_SIZE, DirPerms, FilePerms, HttpMethod, ResolverFn, Sandbox,
    SandboxBuilder, SandboxConfig,
};
use hyperlight_sandbox_pyo3_common::{
    PyExecutionResult, build_tool_registry, parse_size, parse_tool_registration,
};
use hyperlight_wasm_sandbox::Wasm;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

type WasmSandboxInner = Sandbox<Wasm>;
type WasmSnapshotInner = hyperlight_sandbox::Snapshot<<
    <Wasm as hyperlight_sandbox::Guest>::Sandbox as hyperlight_sandbox::GuestSandbox
>::SnapshotData>;

/// Buffered credential registration for lazy sandbox init.
struct PendingCredential {
    id: String,
    target: String,
    header: String,
    prefix: String,
    resolver: ResolverFn,
}

/// Wrap a Python callable as a [`ResolverFn`] suitable for storage in
/// the credential registry.
///
/// On each invocation the wrapper re-acquires the Python GIL, calls
/// the supplied callable with no arguments, and extracts the result
/// as a Python `str`.  Exceptions are mapped to a redacted Rust error
/// — only the exception **type name** is surfaced, never the message
/// (which may contain secret material assembled by user code).
fn python_callable_to_resolver(callable: Py<PyAny>) -> ResolverFn {
    Arc::new(move || -> Result<String, String> {
        Python::attach(|py| {
            let bound = callable.bind(py);
            match bound.call0() {
                Ok(result) => result
                    .extract::<String>()
                    .map_err(|_| "credential resolver did not return a str".to_string()),
                Err(err) => {
                    let type_name = err
                        .get_type(py)
                        .qualname()
                        .ok()
                        .and_then(|n| n.extract::<String>().ok())
                        .unwrap_or_else(|| "Exception".to_string());
                    Err(format!("python resolver raised {type_name}"))
                }
            }
        })
    })
}

#[pyclass]
pub struct PySnapshot {
    inner: WasmSnapshotInner,
}

#[pyclass(unsendable)]
pub struct WasmSandbox {
    inner: Option<WasmSandboxInner>,
    tools: HashMap<String, Py<PyAny>>,
    pending_networks: Vec<(String, Option<Vec<String>>)>,
    pending_credentials: Vec<PendingCredential>,
    config: SandboxConfig,
    input_dir: Option<String>,
    output_dir: Option<String>,
    temp_output: bool,
}

#[pymethods]
impl WasmSandbox {
    #[new]
    #[pyo3(signature = (module_path, input_dir=None, output_dir=None, temp_output=false, heap_size=None, stack_size=None))]
    fn new(
        module_path: &str,
        input_dir: Option<&str>,
        output_dir: Option<&str>,
        temp_output: bool,
        heap_size: Option<&str>,
        stack_size: Option<&str>,
    ) -> PyResult<Self> {
        Ok(WasmSandbox {
            inner: None,
            tools: HashMap::new(),
            pending_networks: Vec::new(),
            pending_credentials: Vec::new(),
            config: SandboxConfig {
                module_path: module_path.to_string(),
                heap_size: match heap_size {
                    Some(s) => parse_size(s)?,
                    None => DEFAULT_HEAP_SIZE,
                },
                stack_size: match stack_size {
                    Some(s) => parse_size(s)?,
                    None => DEFAULT_STACK_SIZE,
                },
            },
            input_dir: input_dir.map(|s| s.to_string()),
            output_dir: output_dir.map(|s| s.to_string()),
            temp_output,
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
                .module_path(&self.config.module_path)
                .heap_size(self.config.heap_size)
                .stack_size(self.config.stack_size)
                .with_tools(registry)
                .guest(Wasm);
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
            let mut sandbox = builder
                .build()
                .map_err(|e| PyRuntimeError::new_err(format!("Failed to create sandbox: {e:#}")))?;
            for (target, methods) in std::mem::take(&mut self.pending_networks) {
                let methods = HttpMethod::parse_list(methods)
                    .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
                sandbox
                    .allow_domain(&target, methods)
                    .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
            }
            for cred in std::mem::take(&mut self.pending_credentials) {
                sandbox
                    .register_credential(
                        cred.id,
                        hyperlight_sandbox::CredentialEntry {
                            target: cred.target,
                            header: cred.header,
                            prefix: cred.prefix,
                            resolver: cred.resolver,
                        },
                    )
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
        if let Some(sandbox) = self.inner.as_mut() {
            let methods = HttpMethod::parse_list(methods)
                .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
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

    /// Register a scoped credential for outgoing HTTP requests.
    ///
    /// Must be called before `run()`. The credential can then be
    /// attached to individual requests by guest code via WIT `attach`.
    ///
    /// `resolver` is a Python callable that takes no arguments and
    /// returns the secret token as a `str`. It is invoked synchronously
    /// from the WASI HTTP dispatch path on every credentialed request,
    /// so it must be fast and thread-safe; long-running token fetches
    /// should be memoised by the caller.
    #[pyo3(signature = (id, target, header, prefix, resolver))]
    fn register_credential(
        &mut self,
        id: &str,
        target: &str,
        header: &str,
        prefix: &str,
        resolver: Py<PyAny>,
    ) -> PyResult<()> {
        let resolver_fn = python_callable_to_resolver(resolver);
        if let Some(sandbox) = self.inner.as_ref() {
            // Register directly on the live sandbox.
            sandbox
                .register_credential(
                    id,
                    hyperlight_sandbox::CredentialEntry {
                        target: target.to_string(),
                        header: header.to_string(),
                        prefix: prefix.to_string(),
                        resolver: resolver_fn,
                    },
                )
                .map_err(|e| PyRuntimeError::new_err(format!("{e}")))?;
        } else {
            // Buffer for later — will be applied when sandbox initialises.
            self.pending_credentials.push(PendingCredential {
                id: id.to_string(),
                target: target.to_string(),
                header: header.to_string(),
                prefix: prefix.to_string(),
                resolver: resolver_fn,
            });
        }
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
fn _native_wasm(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<WasmSandbox>()?;
    m.add_class::<PyExecutionResult>()?;
    m.add_class::<PySnapshot>()?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}
