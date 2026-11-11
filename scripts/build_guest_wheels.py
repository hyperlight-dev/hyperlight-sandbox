"""Build platform-tagged wheels for architecture-specific AOT guests."""

import argparse
import platform
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    machine = platform.machine().lower()
    if sys.platform == "darwin" and machine in {"arm64", "aarch64"}:
        platforms = ["macosx_14_0_arm64"]
    elif sys.platform in {"linux", "win32"} and machine in {"x86_64", "amd64"}:
        # The same x86-64 AOT code runs on Linux and Windows.
        platforms = ["manylinux_2_28_x86_64", "win_amd64"]
    else:
        raise RuntimeError(f"Unsupported guest wheel platform: {sys.platform}/{machine}")

    for tag in platforms:
        subprocess.run(
            [
                sys.executable,
                "-m",
                "build",
                "--wheel",
                f"--config-setting=--build-option=--plat-name={tag}",
                "--outdir",
                str(args.output.resolve()),
            ],
            cwd=args.package,
            check=True,
        )


if __name__ == "__main__":
    main()
