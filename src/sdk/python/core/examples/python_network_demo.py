"""Network access demo for the Python Wasm sandbox."""

from hyperlight_sandbox import Sandbox

try:
    sandbox = Sandbox(backend="wasm", module="python_guest.path")
except ImportError as exc:
    raise SystemExit(
        "This example requires the Wasm backend and packaged Python guest package. "
        "Install hyperlight-sandbox[wasm,python_guest] or run `just python-build`."
    ) from exc

sandbox.allow_domain("https://httpbin.org", methods=["GET"])

# ═══════════════════════════════════════════════════════════════════
# Test 1: Network access denied without permissions
# ═══════════════════════════════════════════════════════════════════
print("═" * 60)
print("Test 1: Network access denied without permissions")
print("═" * 60)
result = sandbox.run("""
try:
    resp = http_get("https://notallowed.example")
    print(f"Got response: {resp['status']}")
except Exception as e:
    print(f"Network blocked: {type(e).__name__}: {e}")
    print("  (notallowed.example is not in the allowlist — correct!)")
""")
print(result.stdout)
assert "Network blocked" in result.stdout, "test 1: expected network access to be blocked"

# ═══════════════════════════════════════════════════════════════════
# Test 2: Network access to allowed domain (WASI-HTTP)
# ═══════════════════════════════════════════════════════════════════
print()
print("═" * 60)
print("Test 2: Network access to allowed domain (WASI-HTTP)")
print("═" * 60)
result = sandbox.run("""
resp = http_get("https://httpbin.org/get")
print(f"HTTP status: {resp['status']}")
print(f"Response body (first 200 chars):")
print(resp['body'][:200])
""")
print(result.stdout)
assert result.success, f"test 2: network access to allowed domain failed\nstderr: {result.stderr[:300]}"

# ═══════════════════════════════════════════════════════════════════
# Test 3: Method filtering — GET allowed, POST blocked
# ═══════════════════════════════════════════════════════════════════
print()
print("═" * 60)
print("Test 3: Method filtering — GET allowed, POST blocked")
print("═" * 60)
result = sandbox.run("""
try:
    resp = http_get("https://httpbin.org/get")
    print(f"GET allowed: status {resp['status']}")
except Exception as e:
    print(f"GET failed: {e}")

try:
    resp = http_post("https://httpbin.org/post", body='{"test": 1}')
    print(f"POST allowed: status {resp['status']}")
except Exception as e:
    print(f"POST blocked: {e}")
    print("  (httpbin.org only allows GET \u2014 correct!)")
""")
print(result.stdout)
assert "POST blocked" in result.stdout, "test 3: expected POST to be blocked"

print("═" * 60)
print("✅ All tests passed!")
print("═" * 60)
