"""Network access demo for the JavaScript Wasm component sandbox (Python SDK)."""

from hyperlight_sandbox import Sandbox

try:
    sandbox = Sandbox(backend="wasm", module="javascript_guest.path")
except ImportError as exc:
    raise SystemExit(
        "This example requires the Wasm backend and packaged JavaScript guest package. "
        "Install hyperlight-sandbox[wasm,javascript_guest] or run `just python-build`."
    ) from exc

sandbox.allow_domain("https://httpbin.org", methods=["GET"])

# ═══════════════════════════════════════════════════════════════════
# Test 1: Network access denied without permissions
# ═══════════════════════════════════════════════════════════════════
print("═" * 60)
print("Test 1: Network access denied without permissions")
print("═" * 60)
result = sandbox.run("""
try {
    const resp = await fetch('https://notallowed.example');
    console.log('Got response: ' + resp.status);
} catch (e) {
    console.log('Network blocked: ' + e.message);
    console.log('  (notallowed.example is not in the allowlist — correct!)');
}
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
const resp = await fetch('https://httpbin.org/get');
const body = await resp.text();
console.log('HTTP status: ' + resp.status);
console.log('Response body (first 200 chars):');
console.log(body.slice(0, 200));
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
try {
    const resp = await fetch('https://httpbin.org/get');
    console.log('GET allowed: status ' + resp.status);
} catch (e) {
    console.log('GET result: ' + e.message);
}
try {
    const resp = await fetch('https://httpbin.org/post', { method: 'POST' });
    console.log('POST allowed: status ' + resp.status);
} catch (e) {
    console.log('POST blocked: ' + e.message);
    console.log('  (httpbin.org only allows GET \u2014 correct!)');
}
""")
print(result.stdout)
assert "POST blocked" in result.stdout, "test 3: expected POST to be blocked"

print("═" * 60)
print("✅ All tests passed!")
print("═" * 60)
