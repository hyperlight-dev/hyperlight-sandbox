use hyperlight_sandbox::{ArgType, ToolRegistry, ToolSchema};
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};

/// Convert a human-readable size string (e.g. `"200Mi"`) to bytes.
pub fn parse_size(size: &str) -> PyResult<u64> {
    let size = size.trim();
    let (value, multiplier) = if let Some(value) = size.strip_suffix("Gi") {
        (value, 1024u64.pow(3))
    } else if let Some(value) = size.strip_suffix("Mi") {
        (value, 1024u64.pow(2))
    } else if let Some(value) = size.strip_suffix("Ki") {
        (value, 1024u64)
    } else {
        (size, 1)
    };
    let parsed: u64 = value
        .parse()
        .map_err(|e| PyRuntimeError::new_err(format!("invalid size: {e}")))?;
    parsed
        .checked_mul(multiplier)
        .ok_or_else(|| PyRuntimeError::new_err("invalid size: value is too large"))
}

/// Wrap an SDK-style tool object (with `.handler` and `.name`) into a plain `**kwargs` callable.
///
/// SAFETY: The Python source below is a compile-time string constant, never
/// constructed from user input.  The user-supplied `tool` object is passed as
/// a function argument to the compiled code, not interpolated into the source,
/// so there is no code-injection vector.
pub fn make_sdk_tool_wrapper(py: Python<'_>, tool: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let wrapper_module = PyModule::from_code(
        py,
        c"
def _make_sdk_wrapper(tool):
    import ast
    import asyncio
    import concurrent.futures

    def _invoke_sync(invocation):
        return asyncio.run(tool.handler(invocation))

    def wrapper(**kwargs):
        invocation = {
            'arguments': kwargs,
            'tool_call_id': 'sandbox',
            'tool_name': tool.name,
        }

        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            loop = None

        if loop and loop.is_running():
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(_invoke_sync, invocation).result(timeout=300)
        else:
            result = _invoke_sync(invocation)

        if isinstance(result, dict) and 'textResultForLlm' in result:
            text = result['textResultForLlm']
            if len(text) < 10_000:
                try:
                    return ast.literal_eval(text)
                except (ValueError, SyntaxError):
                    pass
            return text
        return result

    return wrapper
",
        c"_sdk_tool_wrapper.py",
        c"_sdk_tool_wrapper",
    )?;

    let make_wrapper = wrapper_module.getattr("_make_sdk_wrapper")?;
    Ok(make_wrapper.call1((tool,))?.unbind())
}

/// If `obj` is an awaitable/coroutine, run it to completion and return the result.
/// Otherwise return `obj` unchanged.
pub fn resolve_maybe_coroutine<'py>(
    py: Python<'py>,
    obj: &Bound<'py, PyAny>,
) -> PyResult<Py<PyAny>> {
    let inspect = py.import("inspect")?;
    let is_coro: bool = inspect.call_method1("isawaitable", (obj,))?.extract()?;
    if !is_coro {
        return Ok(obj.clone().unbind());
    }

    let asyncio = py.import("asyncio")?;
    match asyncio.call_method1("run", (obj,)) {
        Ok(result) => return Ok(result.unbind()),
        Err(_) => {}
    }

    let resolver = PyModule::from_code(
        py,
        c"
import asyncio
import concurrent.futures

def _run_coro(coro):
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
        return pool.submit(asyncio.run, coro).result(timeout=300)
",
        c"_coro_resolver.py",
        c"_coro_resolver",
    )?;
    let result = resolver.call_method1("_run_coro", (obj,))?;
    Ok(result.unbind())
}

/// Convert a `serde_json::Value` into a Python object.
pub fn json_to_py(py: Python<'_>, val: &serde_json::Value) -> PyResult<Py<PyAny>> {
    match val {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok((*b).into_pyobject(py)?.to_owned().into_any().unbind()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_pyobject(py)?.into_any().unbind())
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(py.None())
            }
        }
        serde_json::Value::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        serde_json::Value::Array(arr) => {
            let list = pyo3::types::PyList::empty(py);
            for item in arr {
                list.append(json_to_py(py, item)?)?;
            }
            Ok(list.into_any().unbind())
        }
        serde_json::Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, json_to_py(py, v)?)?;
            }
            Ok(dict.into_any().unbind())
        }
    }
}

/// Maximum nesting depth for `py_to_json` to prevent stack overflow.
const MAX_PY_TO_JSON_DEPTH: usize = 64;

/// Convert a Python object into a `serde_json::Value`.
pub fn py_to_json(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    py_to_json_inner(obj, 0)
}

fn py_to_json_inner(obj: &Bound<'_, PyAny>, depth: usize) -> PyResult<serde_json::Value> {
    if depth > MAX_PY_TO_JSON_DEPTH {
        return Err(PyRuntimeError::new_err(format!(
            "py_to_json: maximum nesting depth ({MAX_PY_TO_JSON_DEPTH}) exceeded"
        )));
    }
    if obj.is_none() {
        Ok(serde_json::Value::Null)
    } else if let Ok(b) = obj.extract::<bool>() {
        Ok(serde_json::Value::Bool(b))
    } else if let Ok(i) = obj.extract::<i64>() {
        Ok(serde_json::json!(i))
    } else if let Ok(f) = obj.extract::<f64>() {
        Ok(serde_json::json!(f))
    } else if let Ok(s) = obj.extract::<String>() {
        Ok(serde_json::Value::String(s))
    } else if let Ok(list) = obj.cast::<pyo3::types::PyList>() {
        let arr: PyResult<Vec<serde_json::Value>> = list
            .iter()
            .map(|item| py_to_json_inner(&item, depth + 1))
            .collect();
        Ok(serde_json::Value::Array(arr?))
    } else if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            map.insert(key, py_to_json_inner(&v, depth + 1)?);
        }
        Ok(serde_json::Value::Object(map))
    } else {
        let s = obj.str()?.to_string();
        Ok(serde_json::Value::String(s))
    }
}

/// Result of executing code in a sandbox, exposed to Python.
#[pyclass]
pub struct PyExecutionResult {
    #[pyo3(get)]
    pub stdout: String,
    #[pyo3(get)]
    pub stderr: String,
    #[pyo3(get)]
    pub exit_code: i32,
}

#[pymethods]
impl PyExecutionResult {
    #[getter]
    fn success(&self) -> bool {
        self.exit_code == 0
    }

    fn __repr__(&self) -> String {
        format!(
            "ExecutionResult(exit_code={}, stdout={:?}, stderr={:?})",
            self.exit_code, self.stdout, self.stderr,
        )
    }
}

/// Infer a [`ToolSchema`] from a Python callable's signature.
///
/// Inspects default values to determine expected types:
/// - `int` / `float` defaults → `ArgType::Number`
/// - `str` defaults → `ArgType::String`
/// - `bool` defaults → `ArgType::Boolean`
/// - `dict` defaults → `ArgType::Object`
/// - `list` defaults → `ArgType::Array`
/// - No default → type inferred from annotation if available; otherwise required-untyped
///   with a log warning
///
/// Returns `None` if signature inspection fails (e.g. built-in C function).
pub fn infer_tool_schema(py: Python<'_>, callback: &Bound<'_, PyAny>) -> Option<ToolSchema> {
    let inspect = match py.import("inspect") {
        Ok(m) => m,
        Err(e) => {
            log::debug!("infer_tool_schema: failed to import inspect module: {e}");
            return None;
        }
    };
    let sig = match inspect.call_method1("signature", (callback,)) {
        Ok(s) => s,
        Err(e) => {
            let repr = callback.repr().map(|r| r.to_string()).unwrap_or_default();
            log::debug!("infer_tool_schema: inspect.signature() failed for {repr}: {e}");
            return None;
        }
    };
    let params = sig.getattr("parameters").ok()?;
    let items = params.call_method0("values").ok()?;
    let iter = items.try_iter().ok()?;

    let empty = inspect.getattr("Parameter").ok()?;
    let empty_sentinel = empty.getattr("empty").ok()?;

    let mut schema = ToolSchema::new();
    let mut untyped_args: Vec<std::string::String> = Vec::new();

    for param in iter {
        let param: Bound<'_, PyAny> = match param {
            Ok(p) => p,
            Err(_) => continue,
        };

        let name: String = match param.getattr("name") {
            Ok(n) => match n.extract::<String>() {
                Ok(s) => s,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        // Skip *args / **kwargs.
        let kind: i32 = param
            .getattr("kind")
            .and_then(|k: Bound<'_, PyAny>| k.extract())
            .unwrap_or(-1);
        // VAR_POSITIONAL = 2, VAR_KEYWORD = 4
        if kind == 2 || kind == 4 {
            continue;
        }

        // Try to determine type from annotation first, then from default value.
        let annotation_type = infer_type_from_annotation(&param, &empty_sentinel);

        let default: Bound<'_, PyAny> = match param.getattr("default") {
            Ok(d) => d,
            Err(_) => {
                // No default — required. Use annotation type if we have one.
                if let Some(arg_type) = annotation_type {
                    schema = schema.required_arg(&name, arg_type);
                } else {
                    schema = schema.required_untyped(&name);
                    untyped_args.push(name);
                }
                continue;
            }
        };

        if default.is(&empty_sentinel) {
            // No default value — argument is required.
            if let Some(arg_type) = annotation_type {
                schema = schema.required_arg(&name, arg_type);
            } else {
                schema = schema.required_untyped(&name);
                untyped_args.push(name);
            }
        } else {
            // Has a default value — optional. Prefer annotation type, fall back to default type.
            let arg_type = annotation_type.or_else(|| py_value_to_arg_type(&default));
            if let Some(arg_type) = arg_type {
                schema = schema.optional_arg(&name, arg_type);
            } else {
                untyped_args.push(name);
            }
        }
    }

    if !untyped_args.is_empty() {
        log::warn!(
            "Tool arguments {:?} have no type annotation or typed default — \
             no type validation will be applied. Add type annotations \
             (e.g. `def add(a: float, b: float)`) or typed defaults \
             (e.g. `a=0.0`) to enable validation.",
            untyped_args
        );
    }

    Some(schema)
}

/// Try to infer an `ArgType` from a parameter's type annotation.
///
/// Handles:
/// - Plain types: `int`, `float`, `str`, `bool`, `dict`, `list`
/// - `Annotated[T, ...]`: unwraps to extract `T`, then matches as above
fn infer_type_from_annotation(
    param: &Bound<'_, PyAny>,
    empty_sentinel: &Bound<'_, PyAny>,
) -> Option<ArgType> {
    let annotation = param.getattr("annotation").ok()?;
    if annotation.is(empty_sentinel) {
        return None;
    }

    // Try direct __name__ first (plain type like `float`, `str`).
    if let Some(arg_type) = type_obj_to_arg_type(&annotation) {
        return Some(arg_type);
    }

    // Handle Annotated[T, ...] — unwrap to get the base type T.
    // typing.get_origin(ann) is typing.Annotated → typing.get_args(ann)[0] is T.
    let py = annotation.py();
    if let Ok(typing) = py.import("typing") {
        if let Ok(origin) = typing.call_method1("get_origin", (&annotation,)) {
            // Check if origin is typing.Annotated (available as typing.Annotated since 3.9+)
            if let Ok(annotated_type) = typing.getattr("Annotated") {
                if origin.is(&annotated_type) {
                    if let Ok(args) = typing.call_method1("get_args", (&annotation,)) {
                        // args is a tuple; first element is the base type.
                        if let Ok(base_type) = args.get_item(0) {
                            return type_obj_to_arg_type(&base_type);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Match a Python type object (e.g. `<class 'float'>`) to an `ArgType`.
fn type_obj_to_arg_type(obj: &Bound<'_, PyAny>) -> Option<ArgType> {
    let type_name: std::string::String = obj.getattr("__name__").ok()?.extract().ok()?;
    match type_name.as_str() {
        "int" | "float" => Some(ArgType::Number),
        "str" => Some(ArgType::String),
        "bool" => Some(ArgType::Boolean),
        "dict" => Some(ArgType::Object),
        "list" => Some(ArgType::Array),
        _ => None,
    }
}

/// Map a Python default value to an `ArgType`, or `None` if the type can't be determined.
fn py_value_to_arg_type(val: &Bound<'_, PyAny>) -> Option<ArgType> {
    // Order matters: check bool before int (bool is subclass of int in Python).
    if val.extract::<bool>().is_ok() {
        Some(ArgType::Boolean)
    } else if val.extract::<i64>().is_ok() || val.extract::<f64>().is_ok() {
        Some(ArgType::Number)
    } else if val.extract::<String>().is_ok() {
        Some(ArgType::String)
    } else if val.cast::<pyo3::types::PyDict>().is_ok() {
        Some(ArgType::Object)
    } else if val.cast::<pyo3::types::PyList>().is_ok() {
        Some(ArgType::Array)
    } else {
        None
    }
}

/// Build a [`ToolRegistry`] from a map of Python callables.
///
/// For each tool, infers a schema from the callable's signature and wraps it
/// in a Rust handler that converts JSON to Python kwargs.
pub fn build_tool_registry(
    py: Python<'_>,
    tools: &mut std::collections::HashMap<String, Py<PyAny>>,
) -> PyResult<ToolRegistry> {
    let mut registry = ToolRegistry::new();
    let tools = std::mem::take(tools);
    for (name, callback) in tools {
        let cb = callback.clone_ref(py);
        let schema = infer_tool_schema(py, cb.bind(py)).ok_or_else(|| {
            PyRuntimeError::new_err(format!(
                "Cannot initialize sandbox: unable to infer argument schema for tool '{name}'. \
                 Use a plain function with type annotations or typed defaults \
                 (e.g. `def add(a: float, b: float)` or `lambda a=0, b=0: a + b`)."
            ))
        })?;
        let handler = move |args: serde_json::Value| {
            Python::attach(|py| {
                let kwargs = PyDict::new(py);
                if let serde_json::Value::Object(map) = &args {
                    for (k, v) in map {
                        let py_val = json_to_py(py, v)?;
                        kwargs.set_item(k, py_val)?;
                    }
                }
                let result = cb.call(py, (), Some(&kwargs))?;
                let result = resolve_maybe_coroutine(py, result.bind(py))?;
                py_to_json(result.bind(py))
            })
            .map_err(|e: PyErr| anyhow::anyhow!("{e}"))
        };
        registry.register_with_schema(&name, Some(schema), handler);
    }
    Ok(registry)
}

/// Parse a `(name, callback)` or SDK Tool object into `(name, callable)`.
pub fn parse_tool_registration(
    py: Python<'_>,
    name_or_tool: Py<PyAny>,
    callback: Option<Py<PyAny>>,
) -> PyResult<(String, Py<PyAny>)> {
    let obj = name_or_tool.bind(py);
    if callback.is_none() && obj.hasattr("handler")? && obj.hasattr("name")? {
        let name: String = obj.getattr("name")?.extract()?;
        let wrapper = make_sdk_tool_wrapper(py, obj)?;
        Ok((name, wrapper))
    } else {
        let name: String = obj.extract()?;
        let cb = callback.ok_or_else(|| {
            PyTypeError::new_err("register_tool() expects (name, callable) or a Tool object")
        })?;
        Ok((name, cb))
    }
}
