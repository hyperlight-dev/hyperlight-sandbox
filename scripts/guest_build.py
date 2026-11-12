"""Build guests once for each set of source and compiler inputs."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
from pathlib import Path

WASM_TOOLS_VERSION = "1.252.0"
AOT_VERSION = "0.16.0"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint(root, inputs, context):
    # Git can check out the same sources with LF or CRLF on different hosts.
    values = {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_text(encoding="utf-8").encode("utf-8")).hexdigest()
        for path in sorted(inputs)
    }
    return hashlib.sha256(json.dumps([values, context], sort_keys=True).encode()).hexdigest()


def output_hashes(root, outputs):
    return {path.relative_to(root).as_posix(): digest(path) for path in outputs}


def read_state(path):
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise RuntimeError(f"Invalid build state in {path}; remove this cache entry and rebuild") from error


def write_state(path, state):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(state, sort_keys=True))
    temporary.replace(path)


def cached_build(root, name, inputs, outputs, context, build):
    stamp = root / "target" / "guest-build" / f"{name}.json"
    key = fingerprint(root, inputs, context)
    state = read_state(stamp)
    if (
        state
        and state.get("inputs") == key
        and all(path.is_file() for path in outputs)
        and state.get("outputs") == output_hashes(root, outputs)
    ):
        print(f"Reusing {name}: inputs and output hashes match", flush=True)
        return
    stamp.unlink(missing_ok=True)
    print(f"Building {name}", flush=True)
    build()
    write_state(stamp, {"inputs": key, "outputs": output_hashes(root, outputs)})


class GuestBuilder:
    def __init__(self, root):
        self.root = root
        self.wasm = root / "src" / "wasm_sandbox"
        self.tool_bin = root / "target" / "guest-tools" / "bin"
        self.suffix = ".exe" if os.name == "nt" else ""

    def run(self, *args, cwd=None):
        command = [str(arg) for arg in args]
        command[0] = shutil.which(command[0]) or command[0]
        subprocess.run(command, cwd=cwd or self.root, check=True)

    def version(self, *args):
        command = [shutil.which(args[0]) or args[0], *args[1:]]
        return subprocess.check_output(command, cwd=self.root, text=True).strip()

    def tool(self, name):
        return self.tool_bin / f"{name}{self.suffix}"

    def inputs(self):
        paths = [
            self.root / "scripts" / "guest_build.py",
            self.root / "pyproject.toml",
            self.root / "uv.lock",
            self.root / "Cargo.lock",
            self.root / "rust-toolchain.toml",
        ]
        paths.extend((self.wasm / "wit").rglob("*.wit"))
        python = self.wasm / "guests" / "python"
        paths.extend(python.rglob("*.py"))
        paths.extend(python.rglob("*.pyi"))
        js = self.wasm / "guests" / "javascript"
        paths.extend(js.glob("*.js"))
        paths.extend([js / "package.json", js / "package-lock.json"])
        return paths

    def outputs(self):
        return [
            self.wasm / "wit" / "sandbox-world.wasm",
            self.wasm / "guests" / "python" / "python-sandbox.wasm",
            self.wasm / "guests" / "python" / "python-sandbox.aot",
            self.wasm / "guests" / "javascript" / "js-sandbox.wasm",
            self.wasm / "guests" / "javascript" / "js-sandbox.aot",
        ]

    def tools_context(self):
        return [platform.system(), platform.machine(), self.version("rustc", "-vV")]

    def cache_keys(self):
        context = self.tools_context()
        keys = {
            "tool-key": fingerprint(self.root, [self.root / "scripts" / "guest_build.py"], context),
            "guest-key": fingerprint(
                self.root,
                self.inputs(),
                [
                    *context,
                    self.version("node", "--version"),
                    self.version("npm", "--version"),
                    self.version("uv", "run", "--no-project", "python", "--version"),
                ],
            ),
        }
        for name, value in keys.items():
            print(f"{name}={value}", flush=True)
        if output := os.environ.get("GITHUB_OUTPUT"):
            with open(output, "a", encoding="utf-8") as file:
                file.writelines(f"{name}={value}\n" for name, value in keys.items())

    def tools(self):
        context = self.tools_context()

        def install():
            for name, version in [("wasm-tools", WASM_TOOLS_VERSION), ("hyperlight-wasm-aot", AOT_VERSION)]:
                self.run(
                    "cargo",
                    "install",
                    name,
                    "--locked",
                    "--force",
                    "--version",
                    version,
                    "--root",
                    self.tool_bin.parent,
                    "--target-dir",
                    self.root / "target" / "guest-tool-build",
                )

        cached_build(
            self.root,
            "tools",
            [self.root / "scripts" / "guest_build.py"],
            [self.tool("wasm-tools"), self.tool("hyperlight-wasm-aot")],
            context,
            install,
        )

    def verify_bundle(self):
        state = read_state(self.root / "target" / "guest-build" / "bundle.json")
        if (
            not state
            or state.get("inputs") != fingerprint(self.root, self.inputs(), "guest-bundle")
            or not all(path.is_file() for path in self.outputs())
            or state.get("outputs") != output_hashes(self.root, self.outputs())
        ):
            raise RuntimeError("Guest artifact is missing or does not match this checkout. Rebuild the guest artifact.")
        print("Verified shared guest artifact against checkout and output hashes", flush=True)

    def build(self, kind):
        if kind == "cache-keys":
            self.cache_keys()
            return
        if kind == "verify" or os.environ.get("HYPERLIGHT_GUEST_ARTIFACT") == "1":
            self.verify_bundle()
            return
        if not shutil.which("clang"):
            raise RuntimeError("clang is required; install LLVM before building guests")
        self.tools()
        if kind == "tools":
            return
        context = [digest(self.tool("wasm-tools")), digest(self.tool("hyperlight-wasm-aot"))]
        inputs = self.inputs()
        if kind in ("wit", "all"):
            cached_build(
                self.root,
                "wit",
                inputs,
                self.outputs()[:1],
                context,
                lambda: self.run(
                    self.tool("wasm-tools"),
                    "component",
                    "wit",
                    self.wasm / "wit" / "hyperlight-sandbox.wit",
                    "-w",
                    "-o",
                    self.outputs()[0],
                ),
            )
        if kind in ("python", "all"):
            python_context = [*context, self.version("uv", "run", "--no-project", "python", "--version")]

            def python_guest():
                self.run(
                    "uv",
                    "run",
                    "--frozen",
                    "componentize-py",
                    "-d",
                    "../../wit/hyperlight-sandbox.wit",
                    "-w",
                    "root",
                    "componentize",
                    "sandbox_executor",
                    "-o",
                    "python-sandbox.wasm",
                    cwd=self.wasm / "guests" / "python",
                )
                self.run(self.tool("wasm-tools"), "strip", "--all", self.outputs()[1], "-o", self.outputs()[1])
                self.run(self.tool("hyperlight-wasm-aot"), "compile", "--component", *self.outputs()[1:3])

            cached_build(self.root, "python", inputs, self.outputs()[1:3], python_context, python_guest)
        if kind in ("javascript", "all"):
            js_context = [*context, self.version("node", "--version"), self.version("npm", "--version")]

            def javascript_guest():
                self.run("npm", "ci", cwd=self.wasm / "guests" / "javascript")
                self.run("npm", "run", "build", cwd=self.wasm / "guests" / "javascript")
                self.run(self.tool("hyperlight-wasm-aot"), "compile", "--component", *self.outputs()[3:])

            cached_build(self.root, "javascript", inputs, self.outputs()[3:], js_context, javascript_guest)
        if kind == "all":
            write_state(
                self.root / "target" / "guest-build" / "bundle.json",
                {
                    "inputs": fingerprint(self.root, inputs, "guest-bundle"),
                    "outputs": output_hashes(self.root, self.outputs()),
                },
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["cache-keys", "tools", "wit", "python", "javascript", "all", "verify"])
    GuestBuilder(Path(__file__).resolve().parents[1]).build(parser.parse_args().kind)
