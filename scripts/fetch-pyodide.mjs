/**
 * Fetches the pinned Pyodide engine (engines/pyodide.json) into
 * engines/.cache/pyodide/ — dev/CI-time only, never in the product and never
 * on a recipient's machine (SPEC §5.4: engines come from the reader; a
 * running POID cannot trigger a download).
 *
 * Default mode verifies every file against the pinned sha256 set; `--write`
 * records the pins (the diff is reviewed like code). The extracted layout is
 * exactly what a reader serves as its engine directory.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..");
const manifestPath = join(root, "engines", "pyodide.json");
const cacheDir = join(root, "engines", ".cache");
const engineDir = join(cacheDir, "pyodide");

const write = process.argv.includes("--write");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

const tarball = join(cacheDir, `pyodide-core-${manifest.version}.tar.bz2`);
if (!existsSync(tarball)) {
  console.log(`fetching ${manifest.source}`);
  const res = await fetch(manifest.source);
  if (!res.ok) throw new Error(`${manifest.source}: HTTP ${res.status}`);
  mkdirSync(cacheDir, { recursive: true });
  writeFileSync(tarball, Buffer.from(await res.arrayBuffer()));
}

const tarballDigest = sha256(readFileSync(tarball));
if (write) {
  manifest.sourceSha256 = tarballDigest;
} else if (manifest.sourceSha256 !== tarballDigest) {
  console.error(`engine tarball sha256 ${tarballDigest} != pinned ${manifest.sourceSha256}`);
  process.exit(1);
}

if (!existsSync(join(engineDir, "pyodide.js"))) {
  mkdirSync(engineDir, { recursive: true });
  // tar with bzip2: available on every CI image and on Windows 10+ (bsdtar).
  // Relative paths + cwd keep GNU tar from reading `C:` as a remote host.
  const tar =
    process.platform === "win32" && existsSync("C:\\Windows\\System32\\tar.exe")
      ? "C:\\Windows\\System32\\tar.exe"
      : "tar";
  execFileSync(
    tar,
    ["-xjf", `pyodide-core-${manifest.version}.tar.bz2`, "-C", "pyodide", "--strip-components=1"],
    { cwd: cacheDir, stdio: "inherit" },
  );
}

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) out.push(...walk(path));
    else out.push(path);
  }
  return out;
}

const files = {};
for (const path of walk(engineDir).sort()) {
  const rel = relative(engineDir, path).split("\\").join("/");
  files[rel] = sha256(readFileSync(path));
}

if (write) {
  manifest.files = files;
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(
    `pinned ${Object.keys(files).length} engine files — review and commit engines/pyodide.json`,
  );
} else {
  const pinned = manifest.files;
  let bad = 0;
  for (const [rel, digest] of Object.entries(pinned)) {
    if (files[rel] !== digest) {
      console.error(`FAIL ${rel}: ${files[rel] ?? "missing"} != pinned ${digest}`);
      bad += 1;
    }
  }
  if (bad > 0) process.exit(1);
  console.log(`engine verified: ${Object.keys(pinned).length} files ok in ${engineDir}`);
}
