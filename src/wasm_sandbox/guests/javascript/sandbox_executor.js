/**
 * Guest-side executor that runs inside the Wasm component.
 *
 * Implements hyperlight:sandbox/executor — receives code strings,
 * evaluates them, and returns captured output.
 *
 * File I/O uses WASI filesystem (wasi:filesystem/preopens + types).
 * HTTP uses the standard fetch API (provided by ComponentizeJS).
 * Tools use hyperlight:sandbox/tools WIT interface.
 *
 * Guest code has access to:
 *   call_tool(name, args)              — call a host-registered tool
 *   read_file(path)                    — read text from /input/ or /output/
 *   write_file(path, data)             — write text to /output/
 *   fetch(url, init)                   — standard Fetch API (WASI-HTTP)
 *   console.log(...)                   — captured to stdout
 *   console.error(...)                 — captured to stderr
 */

import { dispatch } from "hyperlight:sandbox/tools";
import { getDirectories } from "wasi:filesystem/preopens@0.2.0";

// ---------- output capture ----------
let stdoutBuf = "";
let stderrBuf = "";
const origLog = console.log;
const origError = console.error;

function captureLog(...args) {
  stdoutBuf += args.map(String).join(" ") + "\n";
}
function captureError(...args) {
  stderrBuf += args.map(String).join(" ") + "\n";
}

// ---------- host helpers ----------

/** Call a host-registered tool via WIT tools.dispatch. */
function callTool(toolName, args = {}) {
  try {
    return JSON.parse(dispatch(toolName, JSON.stringify(args)));
  } catch (e) {
    throw new Error(`Tool '${toolName}' failed: ${e.message || e}`);
  }
}

// ---------- WASI filesystem ----------

let inputDir = null;
let outputDir = null;

function ensurePreopens() {
  // Always re-resolve after snapshot/restore (handles may be stale)
  inputDir = null;
  outputDir = null;
  for (const [desc, path] of getDirectories()) {
    if (path === "/input") inputDir = desc;
    if (path === "/output") outputDir = desc;
  }
}

/** Read a text file via WASI filesystem. */
function readFile(path) {
  ensurePreopens();
  let name = path;
  let dir = inputDir;
  if (name.startsWith("/input/")) {
    name = name.slice(7);
    dir = inputDir;
  } else if (name.startsWith("/output/")) {
    name = name.slice(8);
    dir = outputDir;
  }
  if (!dir) throw new Error("Preopened directory not found for: " + path);
  const fd = dir.openAt({}, name, {}, { read: true });
  const stat = fd.stat();
  const stream = fd.readViaStream(0n);
  const bytes = stream.blockingRead(stat.size);
  return new TextDecoder().decode(bytes);
}

/** Write a text file to /output/ via WASI filesystem. */
function writeFile(path, data) {
  ensurePreopens();
  if (!outputDir) throw new Error("/output/ preopened directory not found");
  let name = path;
  if (name.startsWith("/output/")) name = name.slice(8);
  const fd = outputDir.openAt({}, name, { create: true, truncate: true }, { write: true });
  const stream = fd.writeViaStream(0n);
  stream.blockingWriteAndFlush(new TextEncoder().encode(data));
}

// ---------- executor export ----------

export const executor = {
  async run(code) {
    stdoutBuf = "";
    stderrBuf = "";
    console.log = captureLog;
    console.error = captureError;

    let exitCode = 0;
    try {
      // Use AsyncFunction so user code can `await` async helpers like http_get
      const AsyncFunction = Object.getPrototypeOf(async function(){}).constructor;
      const fn = new AsyncFunction(
        "call_tool", "read_file", "write_file", "console",
        code,
      );
      await fn(
        callTool, readFile, writeFile,
        { log: captureLog, error: captureError, warn: captureError },
      );
    } catch (e) {
      stderrBuf += `${e.name}: ${e.message}\n`;
      exitCode = 1;
    } finally {
      console.log = origLog;
      console.error = origError;
    }

    return {
      stdout: stdoutBuf,
      stderr: stderrBuf,
      exitCode,
    };
  },
};
