/**
 * Builds the browser harness with esbuild and serves it for the Playwright
 * tier. The @poid/sdk sandbox bootstrap is bundled separately to an IIFE and
 * injected into the harness as a string (`__SDK_SOURCE__`), exactly as the
 * real reader injects it into a container.
 */

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

const server = createServer((req, res) => {
  if (req.url === "/harness.js") {
    res.writeHead(200, { "content-type": "text/javascript; charset=utf-8" });
    res.end(harnessJs);
    return;
  }
  res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  res.end(INDEX);
});

server.listen(port, () => {
  console.log(`harness server on http://localhost:${port}`);
});
