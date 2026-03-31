from __future__ import annotations

import importlib
import os
from pathlib import Path

DEFAULT_MODULE_REF = "python_guest.path"


def resolve_module_path(*, module: str | None = None, module_path: str | None = None) -> str:
    if module and module_path:
        raise ValueError("Pass either 'module' or 'module_path', not both.")

    raw_ref = module if module is not None else module_path
    if raw_ref is None:
        raw_ref = DEFAULT_MODULE_REF

    candidate = raw_ref.strip()
    if not candidate:
        raise ValueError("A module reference or module_path is required.")

    path_candidate = Path(candidate).expanduser()
    if path_candidate.exists():
        return str(path_candidate.resolve())

    if any(sep in candidate for sep in (os.sep, "/")) or candidate.endswith((".aot", ".wasm")):
        raise FileNotFoundError(f"Sandbox module path does not exist: {candidate}")

    try:
        module_obj = importlib.import_module(candidate)
    except ImportError as exc:
        raise ImportError(
            f"Unable to resolve sandbox module '{candidate}'. Install the corresponding "
            "guest package or pass an explicit filesystem path."
        ) from exc

    resolved = getattr(module_obj, "MODULE_PATH", None)
    if resolved is None:
        getter = getattr(module_obj, "get_module_path", None)
        if callable(getter):
            resolved = getter()

    if resolved is None:
        raise ValueError(f"Sandbox module '{candidate}' does not expose MODULE_PATH or get_module_path().")

    final_path = Path(str(resolved)).expanduser()
    if not final_path.exists():
        raise FileNotFoundError(f"Resolved sandbox module path does not exist: {final_path}")
    return str(final_path.resolve())
