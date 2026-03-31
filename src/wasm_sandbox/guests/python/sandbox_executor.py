"""Guest-side executor that runs inside the Wasm component.

Phase 2: Code execution with tool dispatch via WIT tools.dispatch import.
Guest code can call host tools via call_tool('name', key=val).

componentize-py expects a class `Executor` matching the WIT `executor` interface,
with a `run` method returning an `ExecutionResult` dataclass.
"""

import sys
import io
import json

from wit_world.exports.executor import ExecutionResult
import wit_world.imports.tools as tools
import wit_world.imports.outgoing_handler as outgoing_handler
import wit_world.imports.wasi_http_types as http_types


def _call_tool(tool_name: str, **kwargs):
    """Call a host-registered tool via WIT tools.dispatch."""
    try:
        result_json = tools.dispatch(tool_name, json.dumps(kwargs))
    except Exception as e:
        raise RuntimeError(f"Tool '{tool_name}' failed: {e}")
    return json.loads(result_json)


def http_get(url: str) -> dict:
    """Make an HTTP GET request via WASI-HTTP. Returns {"status": int, "body": str}."""
    return _http_request("GET", url)


def http_post(url: str, body: str = "", content_type: str = "application/json") -> dict:
    """Make an HTTP POST request via WASI-HTTP. Returns {"status": int, "body": str}."""
    return _http_request("POST", url, body=body, content_type=content_type)


def _http_request(method: str, url: str, body: str = "", content_type: str = "") -> dict:
    """Internal: make an HTTP request via WASI-HTTP outgoing-handler."""
    # Parse URL into scheme, authority, path
    scheme_str, rest = url.split("://", 1) if "://" in url else ("https", url)
    if "/" in rest:
        authority, path = rest.split("/", 1)
        path = "/" + path
    else:
        authority = rest
        path = "/"

    # Build headers
    headers_list = [(b"user-agent", b"hyperlight-sandbox/0.1")]
    if content_type:
        headers_list.append((b"content-type", content_type.encode()))

    fields = http_types.Fields.from_list(
        [(h[0].decode() if isinstance(h[0], bytes) else h[0],
          h[1] if isinstance(h[1], bytes) else h[1].encode())
         for h in headers_list]
    )

    req = http_types.OutgoingRequest(fields)

    # Set method
    method_map = {
        "GET": http_types.Method_Get(),
        "POST": http_types.Method_Post(),
        "PUT": http_types.Method_Put(),
        "DELETE": http_types.Method_Delete(),
        "HEAD": http_types.Method_Head(),
        "OPTIONS": http_types.Method_Options(),
        "PATCH": http_types.Method_Patch(),
    }
    req.set_method(method_map.get(method.upper(), http_types.Method_Get()))

    # Set scheme
    if scheme_str == "https":
        req.set_scheme(http_types.Scheme_Https())
    else:
        req.set_scheme(http_types.Scheme_Http())

    req.set_authority(authority)
    req.set_path_with_query(path)

    # Write body if present
    if body:
        outgoing_body = req.body()
        out_stream = outgoing_body.write()
        out_stream.blocking_write_and_flush(body.encode())
        out_stream.__exit__(None, None, None)
        http_types.OutgoingBody.finish(outgoing_body, None)

    # Send the request
    future_resp = outgoing_handler.handle(req, None)

    # Poll for the response (it's synchronous on the host side)
    pollable = future_resp.subscribe()
    pollable.block()

    resp_result = future_resp.get()
    if resp_result is None:
        raise OSError("HTTP request returned no response")

    # Unwrap Result layers: Ok(Ok(response)) / Ok(Err(ErrorCode)) / None
    resp = resp_result
    if hasattr(resp, 'value'):
        resp = resp.value
    if hasattr(resp, 'value'):
        resp = resp.value

    # Check if we got an ErrorCode instead of a response
    if not hasattr(resp, 'status'):
        raise OSError(f"HTTP request failed: {resp}")

    status = resp.status()
    resp_headers = resp.headers()

    # Read the body
    incoming_body = resp.consume()
    if hasattr(incoming_body, 'value'):
        incoming_body = incoming_body.value
    body_stream = incoming_body.stream()
    if hasattr(body_stream, 'value'):
        body_stream = body_stream.value

    chunks = []
    while True:
        try:
            chunk = body_stream.read(65536)
            if chunk:
                chunks.append(chunk)
            else:
                break
        except Exception:
            break

    body_text = b"".join(chunks).decode("utf-8", errors="replace")

    return {"status": status, "body": body_text}


class Executor:
    """Implements the WIT executor interface for componentize-py."""

    def run(self, code: str) -> ExecutionResult:
        """Execute Python code and capture output."""
        stdout_capture = io.StringIO()
        stderr_capture = io.StringIO()

        old_stdout, old_stderr = sys.stdout, sys.stderr
        sys.stdout = stdout_capture
        sys.stderr = stderr_capture

        exit_code = 0
        try:
            exec(code, {"__builtins__": __builtins__, "call_tool": _call_tool, "http_get": http_get, "http_post": http_post})
        except SystemExit as e:
            exit_code = e.code if isinstance(e.code, int) else 1
        except Exception as e:
            print(f"{type(e).__name__}: {e}", file=stderr_capture)
            exit_code = 1
        finally:
            sys.stdout = old_stdout
            sys.stderr = old_stderr

        return ExecutionResult(
            stdout=stdout_capture.getvalue(),
            stderr=stderr_capture.getvalue(),
            exit_code=exit_code,
        )
