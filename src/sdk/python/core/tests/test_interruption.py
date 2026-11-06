"""Cross-thread guest interruption for both Python SDK backends."""

import threading
import time
import unittest

from hyperlight_sandbox import Sandbox


class TestInterruption(unittest.TestCase):
    def test_idle_handle_and_handle_after_sandbox_drop(self):
        for backend in ("wasm", "hyperlight-js"):
            with self.subTest(backend=backend):
                sandbox = Sandbox(backend=backend)
                handle = sandbox.interrupt_handle()
                self.assertFalse(handle.kill())
                del sandbox
                self.assertFalse(handle.kill())

    def test_interrupt_from_another_thread_and_restore(self):
        for backend in ("wasm", "hyperlight-js"):
            with self.subTest(backend=backend):
                self._interrupt_and_restore(backend)

    def test_pending_host_call_interruption_and_restore(self):
        for backend in ("wasm", "hyperlight-js"):
            with self.subTest(backend=backend):
                attempts = self._interrupt_and_restore(backend, interrupt_during_host_call=True)
                self.assertEqual(attempts, [False])

    def _interrupt_and_restore(self, backend, interrupt_during_host_call=False):
        sandbox = Sandbox(backend=backend)
        started = threading.Event()
        finished = threading.Event()
        release = threading.Event()
        attempts = []

        def ready() -> bool:
            started.set()
            if interrupt_during_host_call:
                return release.wait(5)
            return True

        sandbox.register_tool("ready", ready)
        handle = sandbox.interrupt_handle()
        warmup = "print('ready')" if backend == "wasm" else "console.log('ready');"
        self.assertTrue(sandbox.run(warmup).success)
        snapshot = sandbox.snapshot()

        def interrupt():
            if not started.wait(5):
                return
            if finished.wait(0.05):
                return
            while not finished.is_set():
                attempts.append(handle.kill())
                release.set()
                if attempts[-1] or interrupt_during_host_call:
                    return
                finished.wait(0.01)

        worker = threading.Thread(target=interrupt, daemon=True)
        worker.start()
        code = (
            "import time\ncall_tool('ready')\nend = time.monotonic() + 5\n"
            "while time.monotonic() < end:\n"
            "    for counter in range(100_000):\n        value = counter * counter"
            if backend == "wasm"
            else "call_tool('ready'); const end = Date.now() + 5000; let value = 0;"
            "while (Date.now() < end) {"
            "for (let counter = 0; counter < 100000; counter++) { value = counter * counter; }}"
        )
        start = time.monotonic()
        try:
            with self.assertRaises(InterruptedError):
                sandbox.run(code)
        finally:
            finished.set()
            release.set()
            worker.join(5)
        self.assertFalse(worker.is_alive())
        self.assertTrue(attempts)
        self.assertLess(time.monotonic() - start, 3)
        with self.assertRaises(RuntimeError):
            sandbox.run(warmup)
        sandbox.restore(snapshot)
        self.assertTrue(sandbox.run(warmup).success)
        return attempts

    def test_interruption_does_not_abort_host_callback(self):
        for backend in ("wasm", "hyperlight-js"):
            with self.subTest(backend=backend):
                self._interrupt_host_callback(backend)

    def _interrupt_host_callback(self, backend):
        sandbox = Sandbox(backend=backend)
        entered = threading.Event()
        release = threading.Event()
        finished = threading.Event()
        callback_completed = []
        attempts = []

        def blocked() -> bool:
            entered.set()
            released = release.wait(5)
            callback_completed.append(released)
            return released

        sandbox.register_tool("blocked", blocked)
        handle = sandbox.interrupt_handle()

        def interrupt():
            try:
                if entered.wait(5):
                    attempts.append(handle.kill())
            finally:
                release.set()
            while not finished.is_set():
                if handle.kill():
                    return
                finished.wait(0.01)

        worker = threading.Thread(target=interrupt, daemon=True)
        worker.start()
        code = (
            "import time\ncall_tool('blocked')\nend = time.monotonic() + 5\nwhile time.monotonic() < end:\n    pass"
            if backend == "wasm"
            else "call_tool('blocked'); const end = Date.now() + 5000; while (Date.now() < end) {}"
        )
        try:
            with self.assertRaises(InterruptedError):
                sandbox.run(code)
        finally:
            finished.set()
            release.set()
            worker.join(5)
        self.assertFalse(worker.is_alive())
        self.assertEqual(attempts, [False])
        self.assertEqual(callback_completed, [True])


if __name__ == "__main__":
    unittest.main()
