import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import { promisify } from "node:util";
import { test } from "node:test";

test("builds the JavaScript component with the installed WASI shim", async () => {
  const cwd = new URL(".", import.meta.url);
  await promisify(execFile)(process.execPath, ["build.js"], {
    cwd,
    timeout: 120_000,
  });
  const component = await readFile(new URL("js-sandbox.wasm", cwd));
  assert.deepEqual(component.subarray(0, 8), Buffer.from([0, 97, 115, 109, 13, 0, 1, 0]));
});
