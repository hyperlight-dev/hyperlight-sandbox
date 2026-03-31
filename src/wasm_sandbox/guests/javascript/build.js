import { readFile, writeFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// We bypass ComponentizeJS's stubWasi step which strips WASI filesystem methods
// (openAt, readViaStream, etc.) from the output component. ComponentizeJS has
// no "filesystem" feature, so it stubs those out by default.
//
// Strategy: copy componentize.js next to the original (so relative imports
// resolve), patch the stubWasi call to a no-op, then import the patched copy.
const __dirname = dirname(fileURLToPath(import.meta.url));
const componentizeDir = resolve(
  __dirname,
  "node_modules/@bytecodealliance/componentize-js/src"
);
const originalPath = resolve(componentizeDir, "componentize.js");
const patchedPath = resolve(componentizeDir, "componentize-patched.js");

let src = await readFile(originalPath, "utf-8");
src = src.replace(
  /const finalBin = stubWasi\(\s*bin,\s*features,\s*witWorld,[\s\S]*?worldName\s*\);/,
  "const finalBin = bin; // PATCHED: skip stubWasi to keep all WASI filesystem methods"
);
await writeFile(patchedPath, src);
const { componentize } = await import(patchedPath);

// Use the shared WIT from the wasm_sandbox crate
const witPath = "../../wit/hyperlight-sandbox.wit";
const sourcePath = "sandbox_executor.js";
const outputPath = "js-sandbox.wasm";

async function build() {
  console.log("Reading source...");
  const source = await readFile(sourcePath, "utf-8");

  console.log("Componentizing...");
  const { component, imports } = await componentize(source, {
    witPath,
    worldName: "hyperlight:sandbox/sandbox",
  });

  console.log("Writing component...");
  await writeFile(outputPath, component);

  console.log(`Built ${outputPath} (${component.byteLength} bytes)`);
  if (imports && imports.length > 0) {
    console.log("Imports:", imports);
  }
}

build().catch((err) => {
  console.error("Build failed:", err);
  process.exit(1);
});
