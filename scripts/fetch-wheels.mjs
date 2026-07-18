/**
 * Fetches the Python wheels a project needs into its `deps/` tree
 * (SPEC §5.4, M06 §6): `node scripts/fetch-wheels.mjs <project-dir>`.
 *
 * Authoring-time only, run by the author (running it IS the consent) —
 * the recipient's Pyodide installs from the container's `deps/`, offline,
 * with zero contact with PyPI.
 *
 * Resolution is driven by the *engine's own* lockfile (pyodide-lock.json
 * from the pinned engine, itself checksum-verified): static import scan of
 * the project's .py files → lock packages → dependency closure → wheels,
 * each verified against the lock's sha256.
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");

const projectDir = process.argv[2];
if (!projectDir || !existsSync(projectDir)) {
  console.error("usage: node scripts/fetch-wheels.mjs <project-dir>");
  process.exit(1);
}

const engineManifest = JSON.parse(readFileSync(join(root, "engines", "pyodide.json"), "utf8"));
const lockPath = join(root, "engines", ".cache", "pyodide", "pyodide-lock.json");
if (!existsSync(lockPath)) {
  console.error("engine not fetched — run: node scripts/fetch-pyodide.mjs");
  process.exit(1);
}
const lockBytes = readFileSync(lockPath);
const lockDigest = createHash("sha256").update(lockBytes).digest("hex");
if (lockDigest !== engineManifest.files["pyodide-lock.json"]) {
  console.error("pyodide-lock.json does not match the pinned engine manifest");
  process.exit(1);
}
const lock = JSON.parse(lockBytes.toString("utf8"));

// The wheel repository for this engine release. Pinned by version; every
// file is verified against the lock's sha256, so the CDN is untrusted.
const wheelBase = `https://cdn.jsdelivr.net/pyodide/v${engineManifest.version}/full/`;

function pyFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry !== "deps" && !entry.startsWith(".")) out.push(...pyFiles(path));
    } else if (entry.endsWith(".py")) {
      out.push(path);
    }
  }
  return out;
}

const importNames = new Set();
for (const file of pyFiles(projectDir)) {
  const text = readFileSync(file, "utf8");
  for (const line of text.split("\n")) {
    const m = /^\s*(?:import|from)\s+([A-Za-z_][A-Za-z0-9_]*)/.exec(line);
    if (m) importNames.add(m[1]);
  }
}

// import name → lock package
const byImport = new Map();
for (const pkg of Object.values(lock.packages)) {
  for (const name of pkg.imports ?? []) byImport.set(name, pkg);
}

const selected = new Map();
const queue = [...importNames];
while (queue.length > 0) {
  const name = queue.pop();
  const pkg = byImport.get(name) ?? lock.packages[name];
  if (!pkg) {
    if (importNames.has(name)) {
      console.log(`- ${name}: not a Pyodide package (assuming python stdlib)`);
    }
    continue;
  }
  if (selected.has(pkg.name)) continue;
  selected.set(pkg.name, pkg);
  queue.push(...(pkg.depends ?? []));
}

if (selected.size === 0) {
  console.log("no third-party imports found; nothing to fetch");
  process.exit(0);
}

const depsDir = join(projectDir, "deps");
mkdirSync(depsDir, { recursive: true });
for (const pkg of [...selected.values()].sort((a, b) => a.name.localeCompare(b.name))) {
  const dest = join(depsDir, pkg.file_name);
  if (!existsSync(dest)) {
    const url = `${wheelBase}${pkg.file_name}`;
    console.log(`fetching ${pkg.name}@${pkg.version} (${pkg.file_name})`);
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
  }
  const digest = createHash("sha256").update(readFileSync(dest)).digest("hex");
  if (digest !== pkg.sha256) {
    console.error(`FAIL ${pkg.file_name}: sha256 ${digest} != lock ${pkg.sha256}`);
    process.exit(1);
  }
  console.log(`ok ${pkg.name}@${pkg.version}`);
}
console.log(`deps/ ready: ${selected.size} wheel(s) in ${depsDir}`);
