/**
 * Builds the Studio window bundles into `dist-ui/` (Tauri's `frontendDist`):
 *   - bundles the @poid/sdk sandbox bootstrap to an IIFE and injects it into
 *     the reader bundle as `__SDK_SOURCE__` (the same pattern as the Web
 *     Reader's build-site.mjs),
 *   - bundles src/reader-main.ts → dist-ui/reader.js,
 *   - bundles src/hub-main.ts    → dist-ui/hub.js,
 *   - copies static/ (the two window pages and the stylesheet).
 *
 * Node here is a dev dependency of the repository, never of the product:
 * the output is plain static files served by Tauri from the binary.
 */

import { cpSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = join(here, "..");
const out = join(pkg, "dist-ui");

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

// 1. The sandbox SDK, bundled exactly as the reader injects it (IIFE).
const sdk = await esbuild.build({
  entryPoints: [join(pkg, "..", "..", "packages", "poid-sdk", "src", "bootstrap.ts")],
  bundle: true,
  minify: true,
  format: "iife",
  platform: "browser",
  write: false,
  logLevel: "error",
});
const sdkSource = sdk.outputFiles[0].text;

// 2. The two window bundles.
const common = {
  bundle: true,
  minify: true,
  format: "iife",
  platform: "browser",
  logLevel: "error",
};
await esbuild.build({
  ...common,
  entryPoints: [join(pkg, "src", "reader-main.ts")],
  outfile: join(out, "reader.js"),
  define: { __SDK_SOURCE__: JSON.stringify(sdkSource) },
});
await esbuild.build({
  ...common,
  entryPoints: [join(pkg, "src", "hub-main.ts")],
  outfile: join(out, "hub.js"),
});

// 3. The window pages and the stylesheet.
cpSync(join(pkg, "static"), out, { recursive: true });

console.log(`studio UI built into ${out}`);
