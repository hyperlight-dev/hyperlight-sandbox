"""Full CPython with the data-science stack, via the hyperlight-unikraft backend.

Runs in a Unikraft micro-VM booted from the published ``agent`` rootfs
(CPython 3.12 + numpy, pandas, scipy, scikit-learn). Needs KVM, MSHV or
Hyper-V, and the image in the hluk CLI's cache (``just unikraft pull-images``),
or pass its path with ``module_path=``.
"""

import sys
import time

from hyperlight_sandbox import Sandbox

SALES = [
    {"region": "north", "units": 120, "price": 9.5},
    {"region": "south", "units": 80, "price": 11.0},
    {"region": "north", "units": 45, "price": 9.5},
    {"region": "east", "units": 200, "price": 7.25},
]


def timed(label, fn):
    started = time.perf_counter()
    value = fn()
    print(f"[{label}: {(time.perf_counter() - started) * 1000:.1f} ms]")
    return value


def main() -> None:
    kwargs = {"module_path": sys.argv[1]} if len(sys.argv) > 1 else {"module": "agent"}
    sandbox = Sandbox(backend="unikraft", temp_output=True, **kwargs)
    sandbox.register_tool("fetch_data", lambda table="": SALES if table == "sales" else [])
    sandbox.register_tool("multiply", lambda a=0.0, b=0.0: a * b)

    # The micro-VM boots on the first run.
    result = timed(
        "boot + first run",
        lambda: sandbox.run(
            "import sys, numpy as np\n"
            "print(f'Python {sys.version.split()[0]}, numpy {np.__version__}')\n"
            "print('det =', round(float(np.linalg.det(np.arange(1, 10).reshape(3, 3) + np.eye(3))), 3))"
        ),
    )
    print(result.stdout, end="")
    assert result.success, result.stderr

    sandbox.run("counter = 41")
    result = timed("run", lambda: sandbox.run("counter += 1\nprint('counter =', counter)"))
    print(result.stdout, end="")
    assert "counter = 42" in result.stdout

    result = timed(
        "tools + pandas",
        lambda: sandbox.run(
            "import pandas as pd\n"
            "df = pd.DataFrame(call_tool('fetch_data', table='sales'))\n"
            "df['revenue'] = df['units'] * df['price']\n"
            "print(df.groupby('region')['revenue'].sum().to_string())\n"
            "print('6 x 7 =', call_tool('multiply', a=6, b=7))\n"
            "df.to_csv('/output/sales.csv')"
        ),
    )
    print(result.stdout, end="")
    assert result.success, result.stderr
    print("output files:", sandbox.get_output_files())

    snapshot = timed("snapshot", sandbox.snapshot)
    sandbox.run("counter = 100")
    timed("restore", lambda: sandbox.restore(snapshot))
    result = sandbox.run("print('counter =', counter)")
    print(result.stdout, end="")
    assert "counter = 42" in result.stdout

    result = sandbox.run("1 / 0")
    print("exit code", result.exit_code, "|", result.stderr.strip().splitlines()[-1])


if __name__ == "__main__":
    main()
