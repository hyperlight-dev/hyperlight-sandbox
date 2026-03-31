use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use hyperlight_js::{
    LoadedJSSandbox, SandboxBuilder as JsSandboxBuilder, Script, Snapshot as JsSnapshot,
};
use hyperlight_sandbox::runtime::BlockOn;
use hyperlight_sandbox::{
    CapFs, ExecutionResult, Guest, GuestSandbox, NetworkPermissions, SandboxConfig, Snapshot,
    ToolRegistry, http as sandbox_http,
};
use serde::Deserialize;

const RUN_HANDLER_NAME: &str = "__hyperlight_sandbox_run";
const RUN_HANDLER_SCRIPT: &str = r#"
import * as host from "host";

function __stringify(value) {
    if (typeof value === "string") {
        return value;
    }
    try {
        return JSON.stringify(value);
    } catch (_) {
        return String(value);
    }
}

function __check(raw) {
    const value = JSON.parse(raw);
    if (value && value.error) {
        throw new Error(value.error);
    }
    return value;
}

Object.defineProperty(globalThis, "call_tool", { value: function(name, args = {}) {
    return __check(host.dispatch(name, JSON.stringify(args)));
}, writable: false, configurable: false });

Object.defineProperty(globalThis, "read_file", { value: function(path) {
    const bytes = __check(host.read_file(path));
    let str = "";
    for (let i = 0; i < bytes.length; i += 4096) {
        str += String.fromCharCode.apply(null, bytes.slice(i, i + 4096));
    }
    return str;
}, writable: false, configurable: false });

Object.defineProperty(globalThis, "read_file_bytes", { value: function(path) {
    return new Uint8Array(__check(host.read_file(path)));
}, writable: false, configurable: false });

Object.defineProperty(globalThis, "write_file", { value: function(path, value) {
    if (typeof value === "string") {
        __check(host.write_file_text(path, value));
    } else {
        __check(host.write_file_bytes(path, JSON.stringify(Array.from(value ?? []))));
    }
}, writable: false, configurable: false });

Object.defineProperty(globalThis, "fetch", { value: function(url, init = {}) {
    const options = JSON.stringify({
        method: init.method ?? "GET",
        headers: init.headers ?? {},
        body: init.body ?? null,
    });
    const raw = __check(host.fetch(url, options));
    const bodyText = raw.body ?? "";
    const status = raw.status ?? 0;
    const headers = raw.headers ?? {};
    return Promise.resolve({
        status,
        ok: status >= 200 && status < 300,
        statusText: "",
        headers: {
            get(name) { return headers[name.toLowerCase()] ?? headers[name] ?? null; },
            has(name) { return (name.toLowerCase() in headers) || (name in headers); },
            entries() { return Object.entries(headers); },
        },
        url,
        text() { return Promise.resolve(bodyText); },
        json() { return Promise.resolve(JSON.parse(bodyText)); },
        body: bodyText,
    });
}, writable: false, configurable: false });

function handler(event) {
    try {
        (0, eval)(event);
        return { stderr: "", exit_code: 0 };
    } catch (error) {
        const msg = __stringify(error && error.stack ? error.stack : error);
        return { stderr: msg + "\n", exit_code: 1 };
    }
}

export { handler };
"#;

#[derive(Clone)]
struct HostState {
    tools: Arc<ToolRegistry>,
    files: Arc<Mutex<CapFs>>,
    network: Arc<Mutex<NetworkPermissions>>,
}

#[derive(Deserialize)]
struct RunResponse {
    stderr: String,
    exit_code: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HyperlightJs;

impl Guest for HyperlightJs {
    fn build(
        self,
        config: SandboxConfig,
        tools: ToolRegistry,
        network: std::sync::Arc<std::sync::Mutex<NetworkPermissions>>,
        fs: std::sync::Arc<std::sync::Mutex<CapFs>>,
    ) -> Result<Box<dyn GuestSandbox>> {
        JsGuestSandbox::new(config, tools, network, fs)
            .map(|sandbox| Box::new(sandbox) as Box<dyn GuestSandbox>)
    }
}

struct JsGuestSandbox {
    sandbox: LoadedJSSandbox,
    files: Arc<Mutex<CapFs>>,
    stdout: Arc<Mutex<String>>,
}

#[derive(Deserialize)]
struct FetchOptions {
    #[serde(default = "default_get_method")]
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
}

fn default_get_method() -> String {
    "GET".to_string()
}

impl JsGuestSandbox {
    fn new(
        config: SandboxConfig,
        tools: ToolRegistry,
        network: Arc<Mutex<NetworkPermissions>>,
        files: Arc<Mutex<CapFs>>,
    ) -> Result<Self> {
        // Verify the shared tokio runtime is available before proceeding.
        hyperlight_sandbox::runtime::RUNTIME
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let stdout = Arc::new(Mutex::new(String::new()));
        let tools = Arc::new(tools);
        let host_state = HostState {
            tools,
            files: files.clone(),
            network,
        };

        let print_fn = {
            const MAX_STDOUT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
            let stdout = stdout.clone();
            move |message: String| {
                if let Ok(mut buffer) = stdout.lock() {
                    let remaining = MAX_STDOUT_BYTES.saturating_sub(buffer.len());
                    if remaining > 0 {
                        let take = message.len().min(remaining);
                        buffer.push_str(&message[..take]);
                    }
                }
                0i32
            }
        };

        let mut proto = JsSandboxBuilder::new()
            .with_guest_heap_size(config.heap_size)
            .with_guest_scratch_size(config.stack_size as usize)
            .with_host_print_fn(print_fn.into())
            .build()
            .context("failed to build Hyperlight JS sandbox")?;

        Self::register_host_functions(&mut proto, host_state)?;

        let mut sandbox = proto
            .load_runtime()
            .context("failed to load Hyperlight JS runtime")?;
        sandbox
            .add_handler(RUN_HANDLER_NAME, Script::from_content(RUN_HANDLER_SCRIPT))
            .context("failed to register Hyperlight JS bridge handler")?;

        let loaded = sandbox
            .get_loaded_sandbox()
            .context("failed to load Hyperlight JS bridge handler")?;

        Ok(Self {
            sandbox: loaded,
            files,
            stdout,
        })
    }

    fn register_host_functions(
        proto: &mut hyperlight_js::ProtoJSSandbox,
        state: HostState,
    ) -> Result<()> {
        let tool_state = state.clone();
        proto
            .register(
                "host",
                "dispatch",
                move |name: String, args_json: String| -> String {
                    let args: serde_json::Value = match serde_json::from_str(&args_json) {
                        Ok(args) => args,
                        Err(error) => return json_error(error),
                    };
                    match tool_state.tools.dispatch(&name, args) {
                        Ok(value) => json_response(value),
                        Err(error) => json_error(error),
                    }
                },
            )
            .context("failed to register JS tool dispatch")?;

        let read_state = state.clone();
        proto
            .register("host", "read_file", move |path: String| -> String {
                let Ok(files) = read_state.files.lock() else {
                    return json_error("filesystem mutex poisoned");
                };
                match files.read_guest_file(&path) {
                    Ok(data) => json_response(serde_json::json!(data)),
                    Err(error) => json_error(error),
                }
            })
            .context("failed to register JS file reader")?;

        let write_text_state = state.clone();
        proto
            .register(
                "host",
                "write_file_text",
                move |path: String, text: String| -> String {
                    let Ok(mut files) = write_text_state.files.lock() else {
                        return json_error("filesystem mutex poisoned");
                    };
                    if let Err(error) = files.write_output_path(&path, text.into_bytes()) {
                        return json_error(error);
                    }
                    json_response(serde_json::json!({ "ok": true }))
                },
            )
            .context("failed to register JS text file writer")?;

        let write_bytes_state = state.clone();
        proto
            .register(
                "host",
                "write_file_bytes",
                move |path: String, bytes_json: String| -> String {
                    let bytes: Vec<u8> = match serde_json::from_str(&bytes_json) {
                        Ok(b) => b,
                        Err(error) => return json_error(error),
                    };
                    let Ok(mut files) = write_bytes_state.files.lock() else {
                        return json_error("filesystem mutex poisoned");
                    };
                    if let Err(error) = files.write_output_path(&path, bytes) {
                        return json_error(error);
                    }
                    json_response(serde_json::json!({ "ok": true }))
                },
            )
            .context("failed to register JS bytes file writer")?;

        let http_state = state;
        proto
            .register(
                "host",
                "fetch",
                move |url: String, options_json: String| -> String {
                    let options: FetchOptions = match serde_json::from_str(&options_json) {
                        Ok(opts) => opts,
                        Err(error) => return json_error(error),
                    };

                    let parsed_url = match url::Url::parse(&url) {
                        Ok(u) => u,
                        Err(error) => return json_error(format!("invalid URL: {error}")),
                    };
                    let method: hyperlight_sandbox::HttpMethod = match options.method.parse() {
                        Ok(m) => m,
                        Err(error) => return json_error(error),
                    };

                    {
                        let Ok(network) = http_state.network.lock() else {
                            return json_error("network mutex poisoned");
                        };
                        if !network.is_allowed(&parsed_url, &method) {
                            return json_error(format!(
                                "HTTP request denied for {} {}",
                                method, parsed_url
                            ));
                        }
                    }

                    if options.headers.len() > sandbox_http::MAX_RESPONSE_HEADER_COUNT {
                        return json_error("too many request headers");
                    }

                    let http_req = sandbox_http::HttpRequest {
                        url: parsed_url,
                        method: method.to_string(),
                        headers: options.headers.into_iter().collect(),
                        body: sandbox_http::HttpRequest::body_from_bytes(
                            options.body.map(|b| b.into_bytes()),
                        ),
                    };

                    let resp = match sandbox_http::send_http_request(http_req).block_on() {
                        Ok(r) => r,
                        Err(error) => return json_error(error),
                    };

                    let body = match String::from_utf8(resp.body) {
                        Ok(body) => body,
                        Err(error) => String::from_utf8_lossy(&error.into_bytes()).into_owned(),
                    };

                    json_response(serde_json::json!({
                        "status": resp.status,
                        "headers": resp.headers,
                        "body": body,
                    }))
                },
            )
            .context("failed to register JS HTTP bridge")?;

        Ok(())
    }

    fn run_impl(&mut self, code: &str) -> Result<ExecutionResult> {
        self.stdout
            .lock()
            .map_err(|_| anyhow::anyhow!("stdout mutex poisoned"))?
            .clear();
        self.files
            .lock()
            .map_err(|_| anyhow::anyhow!("filesystem mutex poisoned"))?
            .clear_output_files();

        let request = serde_json::to_string(code)?;

        match self
            .sandbox
            .handle_event(RUN_HANDLER_NAME, request, Some(true))
        {
            Ok(response_json) => {
                let response: RunResponse = serde_json::from_str(&response_json)
                    .context("failed to decode Hyperlight JS run response")?;
                let stdout = std::mem::take(
                    &mut *self
                        .stdout
                        .lock()
                        .map_err(|_| anyhow::anyhow!("stdout mutex poisoned"))?,
                );
                Ok(ExecutionResult {
                    stdout,
                    stderr: response.stderr,
                    exit_code: response.exit_code,
                })
            }
            Err(error) => {
                let mut stderr = error.to_string();
                let stdout = match self.stdout.lock() {
                    Ok(mut s) => std::mem::take(&mut *s),
                    Err(_) => {
                        stderr.push_str("\n[host] failed to read stdout: mutex poisoned");
                        String::new()
                    }
                };
                Ok(ExecutionResult {
                    stdout,
                    stderr,
                    exit_code: -1,
                })
            }
        }
    }
}

impl GuestSandbox for JsGuestSandbox {
    fn run(&mut self, code: &str) -> Result<ExecutionResult> {
        self.run_impl(code)
    }

    fn snapshot(&mut self) -> Result<Snapshot> {
        let runtime = self
            .sandbox
            .snapshot()
            .map_err(|e| anyhow::anyhow!("snapshot failed: {e}"))?;
        Ok(Snapshot::new("hyperlight-js", runtime))
    }

    fn restore(&mut self, snapshot: &Snapshot) -> Result<()> {
        snapshot.restore::<JsSnapshot>(
            |rt| {
                self.sandbox
                    .restore(rt)
                    .map_err(|e| anyhow::anyhow!("restore failed: {e}"))
            },
            &self.files,
        )
    }
}

fn json_error(error: impl std::fmt::Display) -> String {
    json_response(serde_json::json!({ "error": error.to_string() }))
}

fn json_response(value: serde_json::Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|error| {
        format!(
            "{{\"error\":{}}}",
            serde_json::Value::String(error.to_string())
        )
    })
}
