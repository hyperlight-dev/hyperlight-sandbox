"""Regression tests for Python network permission validation."""

import multiprocessing
import os
import sys
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

from hyperlight_sandbox import Sandbox


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        body = b"ok"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format, *args):
        pass


def _serve(connection):
    with HTTPServer(("127.0.0.1", 0), _Handler) as server:
        server.timeout = 0.1
        connection.send(server.server_port)
        while not connection.poll():
            server.handle_request()


def _unikraft_available():
    """Whether the unikraft backend can run here: installed, its python image
    in the hluk cache, and a hypervisor. Like its Rust end-to-end tests, the
    cases skip themselves otherwise."""
    try:
        Sandbox(backend="unikraft", module="python")
    except (ImportError, RuntimeError):
        return False
    if sys.platform == "linux":
        return os.path.exists("/dev/kvm") or os.path.exists("/dev/mshv")
    return sys.platform == "win32"


class TestPendingNetworkPermissions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        context = multiprocessing.get_context("spawn")
        cls.server_connection, child_connection = context.Pipe()
        cls.server_process = context.Process(target=_serve, args=(child_connection,))
        cls.server_process.start()
        cls.url = f"http://127.0.0.1:{cls.server_connection.recv()}"

    @classmethod
    def tearDownClass(cls):
        cls.server_connection.send(None)
        cls.server_process.join()
        cls.server_connection.close()
        cls.server_process.close()

    def backend_cases(self):
        cases = {
            "wasm": {
                "get": f"print(http_get('{self.url}')['status'])",
                "plain": "print('plain')",
            },
            "hyperlight-js": {
                "get": f"const response = await fetch('{self.url}'); console.log(response.status);",
                "plain": "console.log('plain');",
            },
        }
        if _unikraft_available():
            cases["unikraft"] = {
                "get": f"print(http_get('{self.url}')['status'])",
                "plain": "print('plain')",
                "options": {"module": "python"},
            }
        return cases

    def test_valid_queued_rule_works_on_first_run(self):
        for backend, code in self.backend_cases().items():
            with self.subTest(backend=backend):
                sandbox = Sandbox(backend=backend, **code.get("options", {}))
                sandbox.allow_domain(self.url, methods=["GET"])

                result = sandbox.run(code["get"])
                self.assertEqual(result.exit_code, 0, result.stderr)
                self.assertIn("200", result.stdout)

    def test_invalid_queued_method_does_not_discard_valid_rule(self):
        for backend, code in self.backend_cases().items():
            with self.subTest(backend=backend):
                sandbox = Sandbox(backend=backend, **code.get("options", {}))
                sandbox.allow_domain(self.url, methods=["GET"])

                with self.assertRaisesRegex(RuntimeError, "invalid HTTP method: PROPFIND"):
                    sandbox.allow_domain(self.url, methods=["PROPFIND"])

                result = sandbox.run(code["plain"])
                self.assertEqual(result.exit_code, 0, result.stderr)
                self.assertIn("plain", result.stdout)

                result = sandbox.run(code["get"])
                self.assertEqual(result.exit_code, 0, result.stderr)
                self.assertIn("200", result.stdout)

    def test_invalid_method_is_rejected_after_initialization(self):
        for backend, code in self.backend_cases().items():
            with self.subTest(backend=backend):
                sandbox = Sandbox(backend=backend, **code.get("options", {}))

                result = sandbox.run(code["plain"])
                self.assertEqual(result.exit_code, 0, result.stderr)
                self.assertIn("plain", result.stdout)

                with self.assertRaisesRegex(RuntimeError, "invalid HTTP method: PROPFIND"):
                    sandbox.allow_domain(self.url, methods=["PROPFIND"])


if __name__ == "__main__":
    unittest.main()
