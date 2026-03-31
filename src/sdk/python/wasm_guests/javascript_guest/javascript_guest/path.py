from __future__ import annotations

import contextlib
import os
import sys
import tempfile
from importlib import resources
from pathlib import Path


def _default_cache_root() -> Path:
    env = os.environ.get("XDG_CACHE_HOME")
    if env:
        return Path(env)
    if sys.platform == "win32":
        local = os.environ.get("LOCALAPPDATA")
        if local:
            return Path(local)
    elif sys.platform == "darwin":
        return Path.home() / "Library" / "Caches"
    return Path.home() / ".cache"


_CACHE_DIR = _default_cache_root() / "hyperlight_sandbox_guests" / "javascript_guest"


def _materialize(filename: str) -> str:
    resource = resources.files("javascript_guest").joinpath("resources", filename)
    target = _CACHE_DIR / filename
    _CACHE_DIR.mkdir(parents=True, exist_ok=True, mode=0o700)
    data = resource.read_bytes()
    if not target.exists() or target.read_bytes() != data:
        fd, tmp = tempfile.mkstemp(dir=_CACHE_DIR)
        try:
            os.write(fd, data)
            os.close(fd)
            fd = -1
            os.replace(tmp, target)  # atomic on same filesystem
        except BaseException:
            if fd >= 0:
                os.close(fd)
            with contextlib.suppress(OSError):
                os.unlink(tmp)
            raise
    return str(target)


def get_module_path() -> str:
    return _materialize("js-sandbox.aot")


def get_wasm_path() -> str:
    return _materialize("js-sandbox.wasm")


MODULE_PATH = get_module_path()
