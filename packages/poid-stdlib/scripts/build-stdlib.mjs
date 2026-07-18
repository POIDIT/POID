/**
 * Builds the Standard Library from the pinned catalog (ARCHITECTURE §5.2).
 *
 * Dev-time only — this is the ONE place in the repository that talks to the
 * npm registry, and it runs on a maintainer's or CI machine, never in the
 * product and never on a recipient's machine.
 *
 * For every catalog package:
 *   1. fetch the pinned version's metadata from registry.npmjs.org,
 *   2. verify the tarball integrity (sha512 pinned in the catalog),
 *   3. extract into .cache/ (kept for offline rebuilds),
 *   4. bundle every specifier with the pinned esbuild — unminified, so
 *      tree-shaking annotations survive for the app-level build; CJS
 *      packages get a generated named-exports shim (curated in the catalog),
 *   5. sha256 the bundle and verify it against the catalog.
 *
 * `--write` records integrities and checksums into the catalog instead of
 * verifying; the diff is then reviewed and committed. Default mode fails on
 * any mismatch — that is the reproducibility gate CI runs.
 */

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as tar from "tar";

const here = dirname(fileURLToPath(import.meta.url));
const pkgRoot = join(here, "..");
const catalogPath = join(pkgRoot, "src", "catalog.json");
const cacheDir = join(pkgRoot, ".cache");
const libDir = join(pkgRoot, "lib");
const require = createRequire(import.meta.url);

const write = process.argv.includes("--write");
const catalog = JSON.parse(readFileSync(catalogPath, "utf8"));

const esbuildVersion = require("esbuild/package.json").version;
if (esbuildVersion !== catalog.esbuild) {
  console.error(`esbuild ${esbuildVersion} != catalog pin ${catalog.esbuild}`);
  process.exit(1);
}
// On Windows the package's bin entry is a JS launcher (run via node); on
// Linux/macOS the installer replaces it with the native executable itself.
const esbuildBin = require.resolve("esbuild/bin/esbuild");
const esbuildCmd = process.platform === "win32" ? [process.execPath, esbuildBin] : [esbuildBin];

function sha256(buf) {
  return createHash("sha256").update(buf).digest("hex");
}

function sha512b64(buf) {
  return `sha512-${createHash("sha512").update(buf).digest("base64")}`;
}

async function fetchJson(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res.json();
}

/** Downloads (or reuses) one pinned tarball, verifying integrity against
 * `pin` (an object holding `integrity`); extracts it and returns the
 * extracted `package/` directory.
 *
 * The tarball digest is persisted next to the extraction (`.integrity`) so
 * the write/verify logic runs on EVERY invocation, cache hit or not — an
 * early return that skipped it once left closure pins empty in the catalog
 * and broke the first cold-cache CI run. A cached extraction without its
 * provenance file is treated as stale and refetched.
 */
async function materialize(name, version, pin) {
  const safe = name.replace("/", "__");
  const extractDir = join(cacheDir, `${safe}@${version}`);
  const pkgDir = join(extractDir, "package");
  const provenance = join(extractDir, ".integrity");

  if (existsSync(pkgDir) && existsSync(provenance)) {
    const integrity = readFileSync(provenance, "utf8").trim();
    if (write) {
      pin.integrity = integrity;
    } else if (pin.integrity !== integrity) {
      throw new Error(
        `${name}: cached tarball integrity ${integrity} does not match the pinned ${pin.integrity}`,
      );
    }
    return pkgDir;
  }
  rmSync(extractDir, { recursive: true, force: true });

  const meta = await fetchJson(
    `https://registry.npmjs.org/${encodeURIComponent(name).replace("%2F", "/")}/${version}`,
  );
  const url = meta.dist.tarball;
  console.log(`fetching ${name}@${version}`);
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  const bytes = Buffer.from(await res.arrayBuffer());

  const integrity = sha512b64(bytes);
  const registryIntegrity = meta.dist.integrity ?? integrity;
  if (integrity !== registryIntegrity) {
    throw new Error(`${name}: downloaded tarball does not match the registry integrity`);
  }
  if (write) {
    pin.integrity = integrity;
  } else if (pin.integrity !== integrity) {
    throw new Error(
      `${name}: tarball integrity ${integrity} does not match the pinned ${pin.integrity}`,
    );
  }

  mkdirSync(extractDir, { recursive: true });
  const tarball = join(extractDir, "package.tgz");
  writeFileSync(tarball, bytes);
  await tar.x({ file: tarball, cwd: extractDir });
  rmSync(tarball);
  if (!existsSync(pkgDir)) {
    // Not every tarball roots at `package/` — normalize whatever single
    // directory it extracted to.
    const entries = readdirSync(extractDir, { withFileTypes: true }).filter((e) => e.isDirectory());
    if (entries.length !== 1) throw new Error(`${name}: unexpected tarball layout`);
    renameSync(join(extractDir, entries[0].name), pkgDir);
  }
  writeFileSync(provenance, `${integrity}\n`);
  return pkgDir;
}

function parseVer(v) {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$/.exec(v);
  return m ? { n: [Number(m[1]), Number(m[2]), Number(m[3])], pre: m[4] } : undefined;
}

function cmpVer(a, b) {
  for (let i = 0; i < 3; i++) {
    if (a.n[i] !== b.n[i]) return a.n[i] - b.n[i];
  }
  return 0;
}

/** Minimal semver `satisfies` covering the range shapes npm dependencies in
 * this catalog actually use: exact, `^`, `~`, comparators, `x`/`*`, hyphen
 * ranges and `||`. Deliberately no prerelease support — prereleases never
 * enter the Standard Library. */
function satisfies(version, range) {
  const v = parseVer(version);
  if (!v || v.pre) return false;
  return range.split("||").some((alt) => {
    const spaced = alt
      .trim()
      .replace(/\s+-\s+/g, "~~HYPHEN~~")
      // npm allows whitespace between an operator and its version.
      .replace(/(>=|<=|>|<|=|\^|~)\s+/g, "$1");
    const tokens = spaced.split(/\s+/).filter(Boolean);
    return tokens.every((token) => {
      if (token.includes("~~HYPHEN~~")) {
        const [lo, hi] = token.split("~~HYPHEN~~");
        // npm hyphen-range semantics: a partial upper bound is inclusive of
        // its whole prefix (`1 - 3` means `>=1.0.0 <4.0.0`).
        const hiParts = /^(\d+)(?:\.(\d+))?(?:\.(\d+))?$/.exec(hi);
        if (!hiParts) return false;
        let upper;
        if (hiParts[3] !== undefined) {
          upper = `<=${hi}`;
        } else if (hiParts[2] !== undefined) {
          upper = `<${hiParts[1]}.${Number(hiParts[2]) + 1}.0`;
        } else {
          upper = `<${Number(hiParts[1]) + 1}.0.0`;
        }
        return satisfies(version, `>=${lo}`) && satisfies(version, upper);
      }
      if (token === "*" || token === "x" || token === "") return true;
      const op = /^(>=|<=|>|<|=|\^|~)?(.*)$/.exec(token);
      const body = op[2].replace(/\.x$/g, ".0").replace(/\.\*$/g, ".0");
      const partial = /^(\d+)(?:\.(\d+))?(?:\.(\d+))?/.exec(body);
      if (!partial) return false;
      const base = {
        n: [Number(partial[1]), Number(partial[2] ?? 0), Number(partial[3] ?? 0)],
      };
      const c = cmpVer(v, base);
      switch (op[1]) {
        case ">=":
          return c >= 0;
        case "<=":
          return c <= 0;
        case ">":
          return c > 0;
        case "<":
          return c < 0;
        case "~":
          return c >= 0 && v.n[0] === base.n[0] && v.n[1] === base.n[1];
        case "^": {
          if (c < 0) return false;
          if (base.n[0] > 0) return v.n[0] === base.n[0];
          if (base.n[1] > 0) return v.n[0] === 0 && v.n[1] === base.n[1];
          return v.n[0] === 0 && v.n[1] === 0 && v.n[2] === base.n[2];
        }
        default: {
          // Bare or `=`: partial versions act as wildcards (`1.2` = `~1.2`).
          if (partial[3] !== undefined) return c === 0;
          if (partial[2] !== undefined) {
            return v.n[0] === base.n[0] && v.n[1] === base.n[1];
          }
          return v.n[0] === base.n[0];
        }
      }
    });
  });
}

/** Resolves a semver range to the highest matching release (dev-time only;
 * the result is pinned into the catalog and reviewed). */
async function resolveRange(name, range) {
  const doc = await fetchJson(
    `https://registry.npmjs.org/${encodeURIComponent(name).replace("%2F", "/")}`,
  );
  const versions = Object.keys(doc.versions ?? {})
    .map((v) => ({ raw: v, parsed: parseVer(v) }))
    .filter((v) => v.parsed && !v.parsed.pre && satisfies(v.raw, range))
    .sort((a, b) => cmpVer(a.parsed, b.parsed));
  const best = versions[versions.length - 1];
  if (!best) throw new Error(`cannot resolve ${name}@${range}`);
  return best.raw;
}

/**
 * Computes (at `--write`) or reads the pinned dependency closure of a
 * package: every runtime dependency, transitively, EXCEPT names that are
 * themselves Standard Library packages — those stay external and resolve
 * through the catalog at app-build time. One version per name; a conflict is
 * a curation error, not something to silently "solve".
 */
async function closureOf(name, entry) {
  if (!write) return entry.closure ?? {};
  const closure = {};
  const catalogDeps = new Set();
  const queue = [[name, entry.version]];
  const seen = new Set();
  while (queue.length > 0) {
    const [depName, depVersion] = queue.pop();
    if (seen.has(depName)) continue;
    seen.add(depName);
    const meta = await fetchJson(
      `https://registry.npmjs.org/${encodeURIComponent(depName).replace("%2F", "/")}/${depVersion}`,
    );
    for (const [child, range] of Object.entries(meta.dependencies ?? {})) {
      if (catalog.packages[child]) {
        // Served by the catalog: stays a bare import in the bundle, resolved
        // by the app build — every specifier must declare it external.
        catalogDeps.add(child);
        continue;
      }
      const resolved = await resolveRange(child, range);
      if (closure[child] && closure[child].version !== resolved) {
        throw new Error(
          `${name}: dependency conflict on ${child} (${closure[child].version} vs ${resolved})`,
        );
      }
      if (!closure[child]) {
        closure[child] = { version: resolved, integrity: "" };
        queue.push([child, resolved]);
      }
    }
  }
  for (const spec of Object.values(entry.specifiers)) {
    spec.externals = [...new Set([...spec.externals, ...catalogDeps])].sort();
  }
  entry.closure = Object.keys(closure).length > 0 ? closure : undefined;
  return entry.closure ?? {};
}

/** Assembles the build tree for one package: `tree/package` (the package
 * itself) next to `tree/node_modules/<dep>` (its pinned closure), so
 * esbuild's node-style resolution — exports maps included — just works. */
async function assembleTree(name, entry) {
  const pkgDir = await materialize(name, entry.version, entry);
  const closure = await closureOf(name, entry);
  const deps = Object.entries(closure);
  if (deps.length === 0) return pkgDir;

  const safe = name.replace("/", "__");
  const treeDir = join(cacheDir, "trees", `${safe}@${entry.version}`);
  const treePkg = join(treeDir, "package");
  const marker = join(treeDir, ".complete");
  if (!existsSync(marker)) {
    rmSync(treeDir, { recursive: true, force: true });
    mkdirSync(treeDir, { recursive: true });
    cpSync(pkgDir, treePkg, { recursive: true });
    for (const [depName, pin] of deps) {
      const depDir = await materialize(depName, pin.version, pin);
      cpSync(depDir, join(treeDir, "node_modules", depName), { recursive: true });
    }
    writeFileSync(marker, "");
  } else {
    // Verify-mode integrity check still runs for every closure member.
    for (const [depName, pin] of deps) await materialize(depName, pin.version, pin);
  }
  return treePkg;
}

/** Writes the CJS named-exports shim next to the entry and returns its
 * package-relative path. Deterministic content. */
function writeShim(pkgDir, spec, relEntry) {
  const lines = [
    `import __m from "./${relEntry.split("\\").join("/")}";`,
    "export default __m;",
    ...spec.namedExports.map((n) => `export const ${n} = __m.${n};`),
    "",
  ];
  const shimRel = `__poid_shim_${relEntry.split("/").join("_")}.js`;
  writeFileSync(join(pkgDir, shimRel), lines.join("\n"));
  return shimRel;
}

/** Bundles one specifier; returns the output bytes. */
function bundleSpecifier(pkgDir, specifier, spec) {
  const entryRel = spec.namedExports ? writeShim(pkgDir, spec, spec.entry) : spec.entry;
  const outfile = resolve(libDir, `${specifier}.js`);
  mkdirSync(dirname(outfile), { recursive: true });
  const flags = [
    entryRel,
    "--bundle",
    "--format=esm",
    "--platform=browser",
    `--target=${"es2022"}`,
    "--charset=utf8",
    // Licenses of redistributed libraries stay in the bundles.
    "--legal-comments=inline",
    '--define:process.env.NODE_ENV="production"',
    `--outfile=${outfile}`,
  ];
  for (const ext of spec.externals) flags.push(`--external:${ext}`);
  // Node-only imports some libraries reference behind runtime guards
  // (e.g. papaparse's `stream`): aliased to a deterministic empty module.
  for (const stub of spec.stubs ?? []) {
    const stubRel = `__poid_stub_${stub.replace(/[^a-z0-9]/gi, "_")}.js`;
    writeFileSync(join(pkgDir, stubRel), "export default {};\n");
    flags.push(`--alias:${stub}=./${stubRel}`);
  }
  // cwd = the extracted package dir, so path comments in the unminified
  // output are package-relative and byte-identical on every machine.
  const [cmd, ...prefix] = esbuildCmd;
  execFileSync(cmd, [...prefix, ...flags], { cwd: pkgDir, stdio: "pipe" });
  const fixed = fixExternalRequires(readFileSync(outfile, "utf8"), spec.externals);
  writeFileSync(outfile, fixed);
  return Buffer.from(fixed);
}

/**
 * esbuild turns a CommonJS `require()` of an *external* module into a
 * runtime `__require("name")` — which throws in a browser and is opaque to
 * the app-level build. The bundles are unminified, so the pattern is stable:
 * substitute a real static import, which the app build then resolves
 * through the catalog aliases.
 */
function fixExternalRequires(text, externals) {
  let out = text;
  const imports = [];
  externals.forEach((ext, i) => {
    const call = `__require(${JSON.stringify(ext)})`;
    if (out.includes(call)) {
      const id = `__poid_ext_${i}`;
      imports.push(`import ${id} from ${JSON.stringify(ext)};`);
      out = out.split(call).join(id);
    }
  });
  return imports.length > 0 ? `${imports.join("\n")}\n${out}` : out;
}

let failures = 0;
for (const [name, entry] of Object.entries(catalog.packages)) {
  const pkgDir = await assembleTree(name, entry);
  for (const [specifier, spec] of Object.entries(entry.specifiers)) {
    let out;
    try {
      out = bundleSpecifier(pkgDir, specifier, spec);
    } catch (e) {
      console.error(`FAIL ${specifier}: ${String(e.stderr ?? e.message).slice(0, 400)}`);
      failures += 1;
      continue;
    }
    const digest = sha256(out);
    if (write) {
      spec.sha256 = digest;
      console.log(`built ${specifier} (${out.length} bytes, ${digest.slice(0, 12)}…)`);
    } else if (spec.sha256 !== digest) {
      console.error(`FAIL ${specifier}: sha256 ${digest} != pinned ${spec.sha256}`);
      failures += 1;
    } else {
      console.log(`ok ${specifier}`);
    }
  }
}

if (write) {
  writeFileSync(catalogPath, `${JSON.stringify(catalog, null, 2)}\n`);
  console.log("catalog updated — review and commit the diff");
}
if (failures > 0) {
  console.error(`${failures} bundle(s) failed`);
  process.exit(1);
}
console.log(`Standard Library ready in ${libDir}`);
