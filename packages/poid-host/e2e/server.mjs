/**
 * Builds the browser harness with esbuild and serves it for the Playwright
 * tier. The @poid/sdk sandbox bootstrap is bundled separately to an IIFE and
 * injected into the harness as a string (`__SDK_SOURCE__`), exactly as the
 * real reader injects it into a container.
 */

import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const port = Number(process.env.PORT ?? 5178);

async function build() {
  const sdk = await esbuild.build({
    entryPoints: [join(root, "../poid-sdk/src/bootstrap.ts")],
    bundle: true,
    minify: true,
    format: "iife",
    platform: "browser",
    write: false,
    logLevel: "error",
  });
  const sdkSource = sdk.outputFiles[0].text;

  const harness = await esbuild.build({
    entryPoints: [join(here, "harness.ts")],
    bundle: true,
    format: "iife",
    platform: "browser",
    write: false,
    logLevel: "error",
    define: { __SDK_SOURCE__: JSON.stringify(sdkSource) },
  });
  return harness.outputFiles[0].text;
}

const INDEX = `<!doctype html>
<html><head><meta charset="utf-8"><title>POID host harness</title></head>
<body><h1>POID host harness</h1><script src="/harness.js"></script></body></html>`;

const harnessJs = await build();

// The python tier: the verified engine (the reader's asset) and the DoD
// fixture's wheels (the container's deps/ tree). Served only when the engine
// cache exists — the spec skips itself otherwise.
const repoRoot = join(root, "..", "..");
const engineDir = join(repoRoot, "engines", ".cache", "pyodide");
const engineManifestPath = join(repoRoot, "engines", "pyodide.json");
const fixtureDir = join(here, "fixtures", "python-chart");

function serveFile(res, path, type) {
  try {
    const body = readFileSync(path);
    res.writeHead(200, { "content-type": type });
    res.end(body);
  } catch {
    res.writeHead(404, { "content-type": "text/plain" });
    res.end("not found");
  }
}

function typeFor(path) {
  if (path.endsWith(".js") || path.endsWith(".mjs")) return "text/javascript; charset=utf-8";
  if (path.endsWith(".json")) return "application/json; charset=utf-8";
  if (path.endsWith(".wasm")) return "application/wasm";
  return "application/octet-stream";
}

const server = createServer((req, res) => {
  const url = decodeURIComponent((req.url ?? "/").split("?")[0]);
  if (url === "/harness.js") {
    res.writeHead(200, { "content-type": "text/javascript; charset=utf-8" });
    res.end(harnessJs);
    return;
  }
  if (url === "/engine-manifest.json") {
    serveFile(res, engineManifestPath, "application/json; charset=utf-8");
    return;
  }
  if (url.startsWith("/engine/") && !url.includes("..")) {
    serveFile(res, join(engineDir, url.slice("/engine/".length)), typeFor(url));
    return;
  }
  if (url.startsWith("/python-fixture/") && !url.includes("..")) {
    serveFile(res, join(fixtureDir, url.slice("/python-fixture/".length)), typeFor(url));
    return;
  }
  res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  res.end(INDEX);
});

server.listen(port, () => {
  console.log(`harness server on http://localhost:${port}`);
});
