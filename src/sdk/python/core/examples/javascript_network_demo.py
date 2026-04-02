"""Hyperlight Sandbox -- JavaScript network access demo (HyperlightJS backend).

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

# ===================================================================
# Test 1: Network access -- blocked by default
# ===================================================================
print("=" * 60)
print("Test 1: Network access denied without permissions")
print("=" * 60)
result = sandbox.run("""
const resp = await fetch('https://notallowed.example');
if (resp.status === 403) {
    console.log('Network blocked: status ' + resp.status);
    console.log('  (notallowed.example is not in the allowlist -- correct!)');
} else {
    console.log('Got response: ' + resp.status);
}
""")
print(result.stdout)
assert "Network blocked" in result.stdout, "test 1: expected network access to be blocked"

# ===================================================================
# Test 2: Network access -- allowed domain
# ===================================================================
print()
print("=" * 60)
print("Test 2: Network access to allowed domain")
print("=" * 60)
result = sandbox.run("""
const resp = await fetch('https://httpbin.org/get');
const body = await resp.text();
console.log('HTTP status: ' + resp.status);
console.log('Response body (first 200 chars):');
console.log(body.slice(0, 200));
""")
print(result.stdout)
assert result.success, f"test 2: network access to allowed domain failed\nstderr: {result.stderr[:300]}"

# ===================================================================
# Test 3: Method filtering -- GET allowed, POST blocked
# ===================================================================
print()
print("=" * 60)
print("Test 3: Method filtering -- GET allowed, POST blocked")
print("=" * 60)
result = sandbox.run("""
const getResp = await fetch('https://httpbin.org/get');
if (getResp.status === 200) {
    const body = await getResp.text();
    console.log('GET allowed: status ' + getResp.status);
    console.log(body);
} else {
    console.log('GET failed: status ' + getResp.status);
}
const postResp = await fetch('https://httpbin.org/post', { method: 'POST' });
if (postResp.status === 403) {
    console.log('POST blocked: status ' + postResp.status);
    console.log('  (httpbin.org only allows GET -- correct!)');
} else {
    console.log('POST allowed: status ' + postResp.status);
}
""")
print(result.stdout)
assert "POST blocked" in result.stdout, "test 3: expected POST to be blocked"

print("=" * 60)
print("[ok] All tests passed!")
print("=" * 60)
