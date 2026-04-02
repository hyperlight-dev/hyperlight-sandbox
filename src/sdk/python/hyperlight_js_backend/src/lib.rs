use std::collections::HashMap;

use hyperlight_javascript_sandbox::HyperlightJs;
use hyperlight_sandbox::{
    DEFAULT_HEAP_SIZE, DEFAULT_STACK_SIZE, DirPerms, FilePerms, HttpMethod, Sandbox, SandboxConfig,
};
use hyperlight_sandbox_pyo3_common::{
    PyExecutionResult, PySnapshot, build_tool_registry, parse_size, parse_tool_registration,
};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

#[pyclass(unsendable)]
pub struct JSSandbox {
    inner: Option<Sandbox>,
    tools: HashMap<String, Py<PyAny>>,
    pending_networks: Vec<(String, Option<Vec<String>>)>,
    config: SandboxConfig,
    input_dir: Option<String>,
    output_dir: Option<String>,
    temp_output: bool,
}

#[pymethods]
impl JSSandbox {
    #[new]
    #[pyo3(signature = (input_dir=None, output_dir=None, temp_output=false, module_path="", heap_size=None, stack_size=None))]
    fn new(
        input_dir: Option<&str>,
        output_dir: Option<&str>,
        temp_output: bool,
        module_path: &str,
        heap_size: Option<&str>,
        stack_size: Option<&str>,
    ) -> PyResult<Self> {
        if !module_path.is_empty() {
            return Err(PyRuntimeError::new_err(
                "module_path is not supported by the JavaScript backend; \
                 use the Wasm backend if you need a custom module",
            ));
        }
        Ok(JSSandbox {
            inner: None,
            tools: HashMap::new(),
            pending_networks: Vec::new(),
            config: SandboxConfig {
                module_path: String::new(),
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
            let mut builder = Sandbox::builder()
                .module_path(&self.config.module_path)
                .heap_size(self.config.heap_size)
                .stack_size(self.config.stack_size)
                .with_tools(registry)
                .guest(HyperlightJs);
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
fn _native_js(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<JSSandbox>()?;
    m.add_class::<PyExecutionResult>()?;
    m.add_class::<PySnapshot>()?;
    m.add("__version__", "0.1.0")?;
    Ok(())
}
