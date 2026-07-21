/**
 * Builds the deployable static site into `site/`:
 *   - bundles the @poid/sdk sandbox bootstrap to an IIFE and injects it into
 *     the app bundle as `__SDK_SOURCE__` (the same pattern as the desktop
 *     reader and the poid-host e2e harness),
 *   - bundles src/main.ts → site/app.js,
 *   - copies public/ and the generated WASM bindings,
 *   - stamps the service worker with a content hash for cache rotation.
 *
 * Prerequisite: `pnpm --filter @poid/web build:wasm` (needs Rust); everything
 * else is Node-only. The output is plain static files — any CDN can host it.
 */

import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const require = createRequire(import.meta.url);

const here = dirname(fileURLToPath(import.meta.url));
const pkg = join(here, "..");
const site = join(pkg, "site");
const wasmDir = join(pkg, "src", "wasm");

if (!existsSync(join(wasmDir, "poid_wasm.js"))) {
  console.error(
    "site build needs the generated WASM bindings.\n" +
      "Run: pnpm --filter @poid/web build:wasm  (requires a Rust toolchain)\n",
  );
  process.exit(1);
}

rmSync(site, { recursive: true, force: true });
mkdirSync(site, { recursive: true });

// 1. The sandbox SDK, bundled exactly as the reader injects it (IIFE).
const sdk = await esbuild.build({
  entryPoints: [join(pkg, "..", "poid-sdk", "src", "bootstrap.ts")],
  bundle: true,
  minify: true,
  format: "iife",
  platform: "browser",
  write: false,
  logLevel: "error",
});
const sdkSource = sdk.outputFiles[0].text;

// 2. The shell bundle.
await esbuild.build({
  entryPoints: [join(pkg, "src", "main.ts")],
  bundle: true,
  minify: true,
  format: "iife",
  platform: "browser",
  outfile: join(site, "app.js"),
  logLevel: "error",
  define: { __SDK_SOURCE__: JSON.stringify(sdkSource) },
});

// 3. Static assets and the WASM core.
cpSync(join(pkg, "public"), site, { recursive: true });
cpSync(join(wasmDir, "poid_wasm.js"), join(site, "wasm", "poid_wasm.js"));
cpSync(join(wasmDir, "poid_wasm_bg.wasm"), join(site, "wasm", "poid_wasm_bg.wasm"));
// The SQL engine's WASM (M10): fetched lazily on the first poid.db.sql /
// poid.db.docs call, precached by the service worker for offline use.
cpSync(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"), join(site, "wasm", "wa-sqlite.wasm"));

// 4. Stamp the service worker with a digest of the shell, so a new deploy
//    rotates the cache and an unchanged deploy keeps it.
const hash = createHash("sha256");
for (const f of [
  "app.js",
  "index.html",
  "styles.css",
  "wasm/poid_wasm_bg.wasm",
  "wasm/wa-sqlite.wasm",
]) {
  hash.update(readFileSync(join(site, f)));
}
// replaceAll: the placeholder also appears in the file's doc comment, and a
// single replace() would stamp the comment instead of the VERSION constant —
// leaving sw.js byte-identical across deploys, so browsers would never
// refresh the cached shell.
const sw = readFileSync(join(site, "sw.js"), "utf8").replaceAll(
  "__BUILD_HASH__",
  hash.digest("hex").slice(0, 16),
);
writeFileSync(join(site, "sw.js"), sw);

console.log(`site built into ${site}`);
