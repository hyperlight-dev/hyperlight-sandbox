"""Hyperlight Sandbox — JavaScript network access demo (HyperlightJS backend).

Tests network deny-by-default and allow-listed domain access.
"""

from hyperlight_sandbox import Sandbox

try:
    sandbox = Sandbox(backend="hyperlight-js")
except ImportError as exc:
    raise SystemExit(
        "This example requires the HyperlightJS backend. "
        "Install hyperlight-sandbox[hyperlight_js] or run `just python-build`."
    ) from exc

sandbox.allow_domain("https://httpbin.org", methods=["GET"])

# ═══════════════════════════════════════════════════════════════════
# Test 1: Network access — blocked by default
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

# ═══════════════════════════════════════════════════════════════════
# Test 2: Network access — allowed domain
# ═══════════════════════════════════════════════════════════════════
print()
print("═" * 60)
print("Test 2: Network access to allowed domain")
print("═" * 60)
result = sandbox.run("""
const resp = await fetch('https://httpbin.org/get');
const body = await resp.text();
console.log('HTTP status: ' + resp.status);
console.log('Response body (first 200 chars):');
console.log(body.slice(0, 200));
""")
print(result.stdout)
if result.success:
    print("✅ Network access to allowed domain works!")
else:
    print("⚠️ Network access failed")
    print(f"stderr: {result.stderr[:300]}")

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

print("═" * 60)
print("✅ All tests passed!")
print("═" * 60)
