# Guest-side bridge for the hyperlight-unikraft backend.
#
# Run once in the guest's __main__ right after boot. It gives guest code the
# same globals the Wasm Python guest has (call_tool, http_get, http_post) and
# defines the guest function the host calls for every Sandbox::run.
#
# Everything the host hands over travels as a hyperlight host function call,
# which carries at most 64 KiB each way. Replies larger than that arrive as a
# blob id the guest reads back in chunks (see `_hls_unwrap`), requests larger
# than that are uploaded in pieces first (see `_hls_call`), and output is
# flushed to the host in chunks that fit.

import base64 as _hls_b64
import io as _hls_io
import json as _hls_json
import sys as _hls_sys
import traceback as _hls_tb

import hyperlight as _hls_hl

# Characters of output per host call. hyperlight.call sends its arguments as
# ASCII JSON, which spells a character outside the BMP (an emoji) as two
# \uXXXX escapes, 12 bytes: 4096 characters stay under 48 KiB, well within
# the 64 KiB host call limit.
_HLS_CHUNK_CHARS = 4 * 1024

# Largest request, as ASCII JSON, sent in one host call; larger ones are
# uploaded in pieces of half that, since a piece is a JSON string inside the
# call's own JSON, where a quote or backslash doubles.
_HLS_CALL_BYTES = 48 * 1024

# What hyperlight.call raises on a thread other than the one serving the
# call. Any other error is a real failure and must surface.
_HLS_WRONG_THREAD = "only from the thread running a call"


def _hls_call(function, *args):
    """hyperlight.call, for arguments of any size."""
    payload = _hls_json.dumps(list(args))
    if len(payload) <= _HLS_CALL_BYTES:
        return _hls_hl.call(function, *args)
    upload, step = None, _HLS_CALL_BYTES // 2
    for start in range(0, len(payload), step):
        upload = _hls_hl.call("sandbox.upload", upload, payload[start : start + step])
    return _hls_hl.call(function, {"upload": upload})


def _hls_unwrap(reply):
    """Decode a host reply envelope: {"inline": v} or {"blob": id, "len": n}."""
    if "inline" in reply:
        return reply["inline"]
    blob, total = reply["blob"], reply["len"]
    parts, offset = [], 0
    while offset < total:
        chunk = _hls_b64.b64decode(_hls_hl.call("sandbox.blob_read", blob, offset))
        if not chunk:
            raise RuntimeError(f"host blob {blob} ended at {offset} of {total} bytes")
        parts.append(chunk)
        offset += len(chunk)
    return _hls_json.loads(b"".join(parts))


class _HlsHostStream(_hls_io.TextIOBase):
    """A text stream whose writes go to a host function.

    Buffered so a print loop costs one VM exit per chunk rather than per
    call. Only the thread serving a call may make host calls, so a write
    from another thread stays buffered until the serving thread flushes.
    """

    encoding = "utf-8"
    errors = "strict"

    def __init__(self, function):
        super().__init__()
        self._function = function
        self._buffer = []
        self._size = 0

    def writable(self):
        return True

    def isatty(self):
        return False

    def write(self, text):
        if not isinstance(text, str):
            raise TypeError(f"write() argument must be str, not {type(text).__name__}")
        self._buffer.append(text)
        self._size += len(text)
        if self._size >= _HLS_CHUNK_CHARS:
            self.flush()
        return len(text)

    def flush(self):
        if not self._buffer:
            return
        data = "".join(self._buffer)
        self._buffer, self._size = [], 0
        for start in range(0, len(data), _HLS_CHUNK_CHARS):
            try:
                _hls_hl.call(self._function, data[start : start + _HLS_CHUNK_CHARS])
            except RuntimeError as e:
                if _HLS_WRONG_THREAD not in str(e):
                    raise
                # Not the serving thread: keep the rest for the next flush.
                rest = data[start:]
                self._buffer, self._size = [rest], len(rest)
                return


def call_tool(tool_name, **kwargs):
    """Call a host-registered tool by name with keyword arguments."""
    try:
        reply = _hls_call("sandbox.call_tool", tool_name, kwargs)
    except _hls_hl.HostError as e:
        raise RuntimeError(f"Tool '{tool_name}' failed: {e}") from None
    return _hls_unwrap(reply)


def _hls_http(method, url, body="", content_type=""):
    headers = {"user-agent": "hyperlight-sandbox/0.1"}
    if content_type:
        headers["content-type"] = content_type
    try:
        reply = _hls_call("sandbox.http", method, url, headers, body or None)
    except _hls_hl.HostError as e:
        raise OSError(f"HTTP request failed: {e}") from None
    return _hls_unwrap(reply)


def http_get(url):
    """HTTP GET through the host's allowlist. Returns {"status", "headers", "body"}."""
    return _hls_http("GET", url)


def http_post(url, body="", content_type="application/json"):
    """HTTP POST through the host's allowlist. Returns {"status", "headers", "body"}."""
    return _hls_http("POST", url, body=body, content_type=content_type)


# Guest code runs in its own namespace, as in the Wasm guest, so it cannot
# shadow the bridge below; it persists across runs until a restore.
_HLS_GLOBALS = {
    "__name__": "__main__",
    "__builtins__": __builtins__,
    "call_tool": call_tool,
    "http_get": http_get,
    "http_post": http_post,
}


def _hls_exit_status(code, stderr):
    """The status a CPython process would exit with for sys.exit(code)."""
    if code is None:
        return 0
    if isinstance(code, int):  # bool included
        # exit() keeps the low byte; a code past a C long exits with -1.
        return code & 0xFF if -(2**63) <= code < 2**63 else 0xFF
    print(code, file=stderr)
    return 1


def __hyperlight_sandbox_run(request):
    code = _hls_unwrap(request)
    stdout = _HlsHostStream("sandbox.stdout")
    stderr = _HlsHostStream("sandbox.stderr")
    saved = _hls_sys.stdout, _hls_sys.stderr
    _hls_sys.stdout, _hls_sys.stderr = stdout, stderr
    exit_code = 0
    try:
        exec(compile(code, "<sandbox>", "exec"), _HLS_GLOBALS)
    except SystemExit as e:
        exit_code = _hls_exit_status(e.code, stderr)
    except BaseException as e:
        # Drop this function's own frame: the traceback starts in guest code.
        _hls_tb.print_exception(type(e), e, e.__traceback__.tb_next, file=stderr)
        exit_code = 1
    finally:
        _hls_sys.stdout, _hls_sys.stderr = saved
        stdout.flush()
        stderr.flush()
    return exit_code
