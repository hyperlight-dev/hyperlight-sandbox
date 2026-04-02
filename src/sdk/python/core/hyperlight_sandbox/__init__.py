"""Public Python API for Hyperlight Sandbox.

The API package stays stable while backend implementations and packaged guest
modules are installed separately.
"""

from __future__ import annotations

import platform
from collections.abc import Callable
from dataclasses import dataclass, field
from importlib import metadata
from typing import Any

from ._module_resolver import DEFAULT_MODULE_REF, resolve_module_path

try:
    __version__ = metadata.version("hyperlight-sandbox")
except metadata.PackageNotFoundError:
    __version__ = "0.1.0"

# Platform-dependent memory defaults: Windows Hyper-V needs larger allocations.
if platform.system() == "Windows":
    _DEFAULT_HEAP_SIZE = "400Mi"
    _DEFAULT_STACK_SIZE = "200Mi"
else:
    _DEFAULT_HEAP_SIZE = "200Mi"
    _DEFAULT_STACK_SIZE = "100Mi"


def _normalize_backend(backend: str) -> str:
    normalized = backend.strip().lower().replace("_", "-")
    if normalized == "wasm":
        return "wasm"
    if normalized in {"javascript", "js", "hyperlight-js"}:
        return "hyperlight-js"
    raise ValueError(f"Unknown backend '{backend}'. Expected 'wasm' or 'hyperlight-js'.")


def _load_backend(backend: str):
    normalized = _normalize_backend(backend)
    if normalized == "wasm":
        try:
            from hyperlight_sandbox_backend_wasm import WasmSandbox as NativeWasmSandbox
        except ImportError as exc:
            raise ImportError(
                "The Wasm backend is not installed. Install hyperlight-sandbox[wasm] "
                "or install the local hyperlight-sandbox-backend-wasm package."
            ) from exc
        return normalized, NativeWasmSandbox

    try:
        from hyperlight_sandbox_backend_hyperlight_js import JSSandbox as NativeJSSandbox
    except ImportError as exc:
        raise ImportError(
            "The HyperlightJS backend is not installed. Install hyperlight-sandbox[hyperlight_js] "
            "or install the local hyperlight-sandbox-backend-hyperlight-js package."
        ) from exc
    return normalized, NativeJSSandbox


__all__ = [
    "CodeExecutionTool",
    "ExecutionResult",
    "Sandbox",
    "SandboxEnvironment",
    "__version__",
]


@dataclass
class ExecutionResult:
    """Result from code execution in a sandbox."""

    stdout: str
    stderr: str
    exit_code: int

    @property
    def success(self) -> bool:
        return self.exit_code == 0


@dataclass
class SandboxEnvironment:
    """Configuration for creating a sandbox."""

    input_dir: str | None = None
    output_dir: str | None = None
    temp_output: bool = False
    backend: str = "wasm"
    module: str | None = DEFAULT_MODULE_REF
    module_path: str | None = None
    heap_size: str = field(default_factory=lambda: _DEFAULT_HEAP_SIZE)
    stack_size: str = field(default_factory=lambda: _DEFAULT_STACK_SIZE)


class Sandbox:
    """Stable Python API over swappable Hyperlight backends."""

    def __init__(
        self,
        *,
        input_dir: str | None = None,
        output_dir: str | None = None,
        temp_output: bool = False,
        backend: str = "wasm",
        module: str | None = DEFAULT_MODULE_REF,
        module_path: str | None = None,
        heap_size: str | None = None,
        stack_size: str | None = None,
    ) -> None:
        if heap_size is None:
            heap_size = _DEFAULT_HEAP_SIZE
        if stack_size is None:
            stack_size = _DEFAULT_STACK_SIZE
        normalized_backend, native_cls = _load_backend(backend)
        effective_module = module
        if module_path is not None and module == DEFAULT_MODULE_REF:
            effective_module = None

        kwargs: dict[str, Any] = {
            "heap_size": heap_size,
            "stack_size": stack_size,
        }
        if input_dir is not None:
            kwargs["input_dir"] = input_dir
        if output_dir is not None:
            kwargs["output_dir"] = output_dir
        if temp_output:
            kwargs["temp_output"] = True

        if normalized_backend == "wasm":
            resolved_module_path = resolve_module_path(module=effective_module, module_path=module_path)
            self._inner = native_cls(
                module_path=resolved_module_path,
                **kwargs,
            )
        else:
            self._inner = native_cls(**kwargs)

    def register_tool(self, name_or_tool: Any, callback: Any | None = None) -> None:
        """Register a host function callable from sandboxed code.

        Tools must be registered before the first ``run()`` call.
        Registered callbacks are held for the lifetime of the Sandbox
        and cannot be unregistered.  Avoid capturing large object graphs
        in callbacks if memory usage is a concern.
        """
        self._inner.register_tool(name_or_tool, callback)

    # Maximum code size (10 MiB) as defense-in-depth against oversized payloads.
    MAX_CODE_SIZE: int = 10 * 1024 * 1024

    def run(self, code: str) -> ExecutionResult:
        if len(code) > self.MAX_CODE_SIZE:
            raise ValueError(f"code exceeds maximum size ({len(code)} > {self.MAX_CODE_SIZE} bytes)")
        native_result = self._inner.run(code)
        return ExecutionResult(
            stdout=native_result.stdout,
            stderr=native_result.stderr,
            exit_code=native_result.exit_code,
        )

    def get_output_files(self) -> list[str]:
        """List filenames written by the guest to the output directory.

        Returns a list of filenames (not contents). Use output_path()
        to get the host directory and read files directly from disk.
        """
        return list(self._inner.get_output_files())

    def output_path(self) -> str | None:
        """Return the host filesystem path to the output directory.

        Returns None if no output directory was configured.
        """
        return self._inner.output_path()

    def allow_domain(self, target: str, methods: list[str] | None = None) -> None:
        self._inner.allow_domain(target, methods)

    def snapshot(self):
        """Capture the current sandbox state.

        Returns a snapshot object backed by shared reference counting.
        Old snapshots should be deleted when no longer needed to allow
        memory reclamation.
        """
        return self._inner.snapshot()

    def restore(self, snapshot: Any) -> None:
        self._inner.restore(snapshot)


@dataclass
class CodeExecutionTool:
    """High-level tool for agent framework integration."""

    environment: SandboxEnvironment = field(default_factory=SandboxEnvironment)
    tools: list[Callable[..., Any]] = field(default_factory=list)

    _sandbox: Sandbox | None = field(default=None, init=False, repr=False)

    def _get_sandbox(self) -> Sandbox:
        if self._sandbox is None:
            kwargs: dict[str, Any] = {}
            if self.environment.input_dir is not None:
                kwargs["input_dir"] = self.environment.input_dir
            if self.environment.output_dir is not None:
                kwargs["output_dir"] = self.environment.output_dir
            if self.environment.temp_output:
                kwargs["temp_output"] = True
            self._sandbox = Sandbox(
                **kwargs,
                backend=self.environment.backend,
                module=self.environment.module,
                module_path=self.environment.module_path,
                heap_size=self.environment.heap_size,
                stack_size=self.environment.stack_size,
            )
            for tool_fn in self.tools:
                self._sandbox.register_tool(tool_fn)
        return self._sandbox

    def run(
        self,
        code: str,
        inputs: dict[str, bytes] | None = None,
        outputs: list[str] | None = None,
    ) -> ExecutionResult:
        del inputs, outputs
        sandbox = self._get_sandbox()
        return sandbox.run(code)
