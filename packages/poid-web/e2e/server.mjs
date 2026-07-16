/**
 * Builds the static site and serves it for the Playwright tier — the local
 * stand-in for the CDN. It serves files and nothing else: no compute, no
 * state, exactly what production hosting provides.
 *
 * Prerequisite: the WASM bindings (`build:wasm`) must already be generated;
 * the site build fails with instructions otherwise.
 */

import { spawnSync } from "node:child_process";
import { createReadStream, existsSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = join(here, "..");
const site = join(pkg, "site");
const port = Number(process.env.PORT ?? 5179);

const build = spawnSync(process.execPath, [join(pkg, "scripts", "build-site.mjs")], {
  stdio: "inherit",
});
if (build.status !== 0) process.exit(build.status ?? 1);

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".wasm": "application/wasm",
  ".poid": "application/vnd.poid+zip",
};

const server = createServer((req, res) => {
  const path = normalize(decodeURIComponent((req.url ?? "/").split("?")[0])).replace(/^[/\\]+/, "");
  let file = join(site, path === "" ? "index.html" : path);
  if (!file.startsWith(site)) {
    res.writeHead(403).end();
    return;
  }
  if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
  if (!existsSync(file)) {
    res.writeHead(404, { "content-type": "text/plain" }).end("not found");
    return;
  }
  res.writeHead(200, {
    "content-type": MIME[extname(file)] ?? "application/octet-stream",
  });
  createReadStream(file).pipe(res);
});

server.listen(port, () => {
  console.log(`web reader on http://localhost:${port}`);
});
