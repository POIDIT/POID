/**
 * Stages the pinned esbuild engine (engines/esbuild.json) into
 * engines/.cache/esbuild/ — dev/CI-time only, never in the product and never
 * on a recipient's machine.
 *
 * Unlike fetch-pyodide, this does **not** hit the network: `esbuild-wasm` is
 * already a dev dependency of `@poid/toolchain`, so the wasm is on disk in
 * node_modules. Staging it here, verified against the pin, gives Studio's e2e
 * a local engine directory that stands in for the Cloudflare download the real
 * product performs. `--write` records the pin from whatever version is
 * installed (the diff is reviewed like code).
 *
 * The wasm the product downloads at runtime is the *same bytes* this stages;
 * the pin in engines/esbuild.json is the single source of truth for both.
 */

import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const manifestPath = join(root, "engines", "esbuild.json");
const engineDir = join(root, "engines", ".cache", "esbuild");

const write = process.argv.includes("--write");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

// The one file the engine loads. Resolved from `@poid/toolchain`, which
// depends on esbuild-wasm directly, so the staged bytes and the version the
// toolchain bundles against cannot drift. esbuild-wasm has no `exports` map,
// so resolve the package's own entry and take the sibling wasm rather than a
// bare subpath (which pnpm will not map).
const require = createRequire(join(root, "packages", "poid-toolchain", "package.json"));
const wasmSource = join(dirname(require.resolve("esbuild-wasm")), "..", "esbuild.wasm");
const wasm = readFileSync(wasmSource);
const digest = sha256(wasm);

if (!existsSync(engineDir)) mkdirSync(engineDir, { recursive: true });
copyFileSync(wasmSource, join(engineDir, "esbuild.wasm"));

if (write) {
  manifest.files = { "esbuild.wasm": digest };
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(
    `pinned esbuild.wasm (${wasm.length} bytes) — review and commit engines/esbuild.json`,
  );
} else {
  const pinned = manifest.files["esbuild.wasm"];
  if (digest !== pinned) {
    console.error(`FAIL esbuild.wasm: ${digest} != pinned ${pinned}`);
    console.error("The installed esbuild-wasm differs from engines/esbuild.json.");
    console.error("If the version bump is intended, run with --write and review the diff.");
    process.exit(1);
  }
  console.log(`engine verified: esbuild.wasm ok in ${engineDir}`);
}
