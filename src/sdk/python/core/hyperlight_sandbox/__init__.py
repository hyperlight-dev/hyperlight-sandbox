"""Public Python API for Hyperlight Sandbox.

The API package stays stable while backend implementations and packaged guest
modules are installed separately.
"""

from __future__ import annotations

import platform
from collections.abc import Callable
from dataclasses import dataclass, field
from importlib import metadata
from typing import Any, Literal

from ._module_resolver import DEFAULT_MODULE_REF, resolve_module_path

try:
    __version__ = metadata.version("hyperlight-sandbox")
except metadata.PackageNotFoundError:
    __version__ = "0.3.0"

# Platform-dependent memory defaults: Windows Hyper-V needs larger allocations.
if platform.system() == "Windows":
    _DEFAULT_HEAP_SIZE = "400Mi"
    _DEFAULT_STACK_SIZE = "200Mi"
else:
    _DEFAULT_HEAP_SIZE = "25Mi"
    _DEFAULT_STACK_SIZE = "35Mi"


def _normalize_backend(backend: str) -> str:
    normalized = backend.strip().lower().replace("_", "-")
    if normalized == "wasm":
        return "wasm"
    if normalized in {"javascript", "js", "hyperlight-js"}:
        return "hyperlight-js"
    if normalized in {"unikraft", "hyperlight-unikraft", "hluk"}:
        return "unikraft"
    raise ValueError(f"Unknown backend '{backend}'. Expected 'wasm', 'hyperlight-js' or 'unikraft'.")


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

    if normalized == "unikraft":
        try:
            from hyperlight_sandbox_backend_unikraft import UnikraftSandbox as NativeUnikraftSandbox
        except ImportError as exc:
            raise ImportError(
                "The hyperlight-unikraft backend is not installed. Install hyperlight-sandbox[unikraft] "
                "or install the local hyperlight-sandbox-backend-unikraft package."
            ) from exc
        return normalized, NativeUnikraftSandbox

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
    """Configuration for creating a sandbox.

    Filesystem quotas use human-readable binary size strings such as ``"64Mi"``.
    Leave all quota fields unset to use balanced backend defaults, or set
    ``filesystem_limits="unlimited"`` to explicitly disable filesystem quotas.
    """

    input_dir: str | None = None
    output_dir: str | None = None
    temp_output: bool = False
    backend: str = "wasm"
    module: str | None = DEFAULT_MODULE_REF
    module_path: str | None = None
    # None: the backend's default (the unikraft backend takes none of them).
    heap_size: str | None = None
    stack_size: str | None = None
    filesystem_limits: Literal["unlimited"] | None = None
    max_file_size: str | None = None
    max_total_size: str | None = None
    max_file_count: int | None = None
    scratch_size: str | None = None
    native_network: bool = False


class Sandbox:
    """Stable Python API over swappable Hyperlight backends.

    ``max_file_size`` and ``max_total_size`` accept binary size strings such as
    ``"64Mi"``. Partial numeric overrides retain balanced defaults for omitted
    values. Use ``filesystem_limits="unlimited"`` to explicitly disable all
    filesystem quotas; it cannot be combined with numeric overrides.

    ``backend="unikraft"`` runs full CPython in a hyperlight-unikraft micro-VM.
    ``module_path`` is then a rootfs image (initrd CPIO), and ``module`` names a
    published image the ``hluk`` CLI has cached (``"agent"`` by default: CPython
    with numpy, pandas, scipy and scikit-learn; or ``"python"``). ``heap_size``
    and ``stack_size`` are refused for it: ``scratch_size`` (e.g. ``"2Gi"``)
    overrides the guest memory the image was tested with. ``native_network``
    also lets guest sockets (``requests``, ``urllib``) reach the domains passed
    to ``allow_domain``, enforced by host name only; a name that does not
    resolve is left out, and the next run's stderr says so. ``/input`` and
    ``/output`` are host directories mounted at boot; a run that leaves a
    symbolic or hard link in ``/output`` fails, as one past the filesystem
    limits does. Before each run ``/output`` is cleared of every file and
    link and of the directories the guest made; those it had when the
    sandbox was built are kept. Exit statuses are a CPython process's, 0 to
    255 (``sys.exit(-1)`` is 255), so -1 means the guest itself failed.
    Tool arguments and results and HTTP bodies of any size are exchanged in
    pieces, up to 64 MiB held unread on the host at a time.
    """

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
        filesystem_limits: Literal["unlimited"] | None = None,
        max_file_size: str | None = None,
        max_total_size: str | None = None,
        max_file_count: int | None = None,
        scratch_size: str | None = None,
        native_network: bool = False,
    ) -> None:
        numeric_limits = (max_file_size, max_total_size, max_file_count)
        if filesystem_limits is not None and filesystem_limits != "unlimited":
            raise ValueError("filesystem_limits must be 'unlimited' when provided")
        if filesystem_limits == "unlimited" and any(value is not None for value in numeric_limits):
            raise ValueError(
                "filesystem_limits='unlimited' cannot be combined with max_file_size, max_total_size, or max_file_count"
            )
        for name, value in (
            ("max_file_size", max_file_size),
            ("max_total_size", max_total_size),
        ):
            if value is not None and not isinstance(value, str):
                raise TypeError(f"{name} must be a size string")
        if max_file_count is not None:
            if isinstance(max_file_count, bool) or not isinstance(max_file_count, int):
                raise TypeError("max_file_count must be an integer")
            if max_file_count < 0:
                raise ValueError("max_file_count must be non-negative")

        # Checked before the backend is imported, so a wrong option is a
        # ValueError whichever backends are installed.
        unikraft = _normalize_backend(backend) == "unikraft"
        if unikraft and (heap_size is not None or stack_size is not None):
            raise ValueError("heap_size and stack_size do not apply to the unikraft backend; use scratch_size")
        if not unikraft and (scratch_size is not None or native_network):
            raise ValueError("scratch_size and native_network apply only to the unikraft backend")
        if unikraft and module_path is not None and module not in (None, DEFAULT_MODULE_REF):
            raise ValueError("pass either module or module_path, not both")
        if heap_size is None:
            heap_size = _DEFAULT_HEAP_SIZE
        if stack_size is None:
            stack_size = _DEFAULT_STACK_SIZE
        normalized_backend, native_cls = _load_backend(backend)
        effective_module = module
        if module_path is not None and module == DEFAULT_MODULE_REF:
            effective_module = None

        kwargs: dict[str, Any] = {}
        if unikraft:
            if scratch_size is not None:
                kwargs["scratch_size"] = scratch_size
            if native_network:
                kwargs["native_network"] = True
        else:
            kwargs["heap_size"] = heap_size
            kwargs["stack_size"] = stack_size
        if input_dir is not None:
            kwargs["input_dir"] = input_dir
        if output_dir is not None:
            kwargs["output_dir"] = output_dir
        if temp_output:
            kwargs["temp_output"] = True
        if filesystem_limits is not None:
            kwargs["filesystem_limits"] = filesystem_limits
        if max_file_size is not None:
            kwargs["max_file_size"] = max_file_size
        if max_total_size is not None:
            kwargs["max_total_size"] = max_total_size
        if max_file_count is not None:
            kwargs["max_file_count"] = max_file_count

        if normalized_backend == "wasm":
            resolved_module_path = resolve_module_path(module=effective_module, module_path=module_path)
            self._inner = native_cls(
                module_path=resolved_module_path,
                **kwargs,
            )
        elif normalized_backend == "unikraft":
            # The Wasm default module names a Wasm guest package; for this
            # backend a module is a published rootfs image name.
            if module_path is not None:
                kwargs["module_path"] = module_path
            elif effective_module is not None and effective_module != DEFAULT_MODULE_REF:
                kwargs["runtime"] = effective_module
            self._inner = native_cls(**kwargs)
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
                filesystem_limits=self.environment.filesystem_limits,
                max_file_size=self.environment.max_file_size,
                max_total_size=self.environment.max_total_size,
                max_file_count=self.environment.max_file_count,
                scratch_size=self.environment.scratch_size,
                native_network=self.environment.native_network,
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
