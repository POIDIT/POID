/**
 * The reproducibility proofs behind ARCHITECTURE §5.1:
 *
 * 1. the native esbuild engine and esbuild-wasm produce **byte-identical**
 *    bundles for the same input (the CLI path and the in-app path agree),
 * 2. the contract's CLI flags mean the same thing as its JS-API options
 *    (what `crates/poid-cli` passes to the sidecar equals what the app
 *    passes to esbuild-wasm),
 * 3. building twice produces the same bytes (determinism over time).
 */

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import * as esbuildNative from "esbuild";
import * as esbuildWasm from "esbuild-wasm";
import { afterAll, describe, expect, it } from "vitest";
import { contractCliFlags, PINNED_ESBUILD } from "./contract.js";
import { type BundleInput, bundle, type EsbuildApi } from "./engine.js";

const require = createRequire(import.meta.url);
const encoder = new TextEncoder();

/** A small project exercising TS, CSS, CSS modules, JSON and asset inlining. */
const FIXTURE: Record<string, string> = {
  "src/main.ts": [
    'import { greet } from "./lib/greet";',
    'import config from "./config.json";',
    'import styles from "./app.module.css";',
    'import "./global.css";',
    'import icon from "./icon.svg";',
    'import notes from "./notes.txt";',
    "const el = document.createElement('div');",
    "el.className = styles.card;",
    "el.textContent = `${greet(config.name)} ${notes.length}`;",
    "el.style.backgroundImage = `url(${icon})`;",
    "document.body.append(el);",
  ].join("\n"),
  "src/lib/greet.ts": "export function greet(name: string): string { return `hello ${name}`; }",
  "src/config.json": '{ "name": "poid" }',
  "src/app.module.css": ".card { border: 1px solid rebeccapurple; }",
  "src/global.css": "body { margin: 0; font-family: system-ui; }",
  "src/icon.svg": '<svg xmlns="http://www.w3.org/2000/svg"><rect width="4" height="4"/></svg>',
  "src/notes.txt": "three\nlines\nhere",
};

function fixtureMap(): Map<string, Uint8Array> {
  return new Map(Object.entries(FIXTURE).map(([k, v]) => [k, encoder.encode(v)]));
}

const input: BundleInput = { files: fixtureMap(), entry: "src/main.ts" };

const tempDirs: string[] = [];
afterAll(() => {
  for (const dir of tempDirs) rmSync(dir, { recursive: true, force: true });
});

/** Writes the fixture to disk and builds it with the real sidecar flags, the
 * way `crates/poid-cli` drives the native binary. */
function buildViaCliFlags(): { js: Buffer; css: Buffer } {
  const dir = mkdtempSync(join(tmpdir(), "poid-parity-"));
  tempDirs.push(dir);
  for (const [rel, content] of Object.entries(FIXTURE)) {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, content);
  }
  const bin = require.resolve("esbuild/bin/esbuild");
  const result = spawnSync(
    process.execPath,
    [bin, "src/main.ts", ...contractCliFlags(), "--outdir=out"],
    { cwd: dir },
  );
  expect(result.status, String(result.stderr)).toBe(0);
  return {
    js: readFileSync(join(dir, "out", "main.js")),
    css: readFileSync(join(dir, "out", "main.css")),
  };
}

describe("build parity", () => {
  it("pins both engines to the contract version", () => {
    expect(esbuildNative.version).toBe(PINNED_ESBUILD);
    expect(esbuildWasm.version).toBe(PINNED_ESBUILD);
  });

  // Mirrors `contract_flags_match_the_toolchain_side` in
  // crates/poid-cli/src/project.rs — the same literal list on both sides is
  // the tripwire against silent contract drift.
  it("generates the exact flag sequence the CLI side pins", () => {
    expect(contractCliFlags()).toEqual([
      "--bundle",
      "--format=esm",
      "--platform=browser",
      "--target=es2022",
      "--charset=utf8",
      "--legal-comments=none",
      "--jsx=automatic",
      "--minify",
      "--entry-names=main",
      '--define:process.env.NODE_ENV="production"',
      "--loader:.avif=dataurl",
      "--loader:.csv=text",
      "--loader:.gif=dataurl",
      "--loader:.ico=dataurl",
      "--loader:.jpeg=dataurl",
      "--loader:.jpg=dataurl",
      "--loader:.md=text",
      "--loader:.mp3=dataurl",
      "--loader:.otf=dataurl",
      "--loader:.png=dataurl",
      "--loader:.svg=dataurl",
      "--loader:.ttf=dataurl",
      "--loader:.txt=text",
      "--loader:.wav=dataurl",
      "--loader:.webp=dataurl",
      "--loader:.woff=dataurl",
      "--loader:.woff2=dataurl",
    ]);
  });

  it("native and wasm engines produce byte-identical bundles", async () => {
    const native = await bundle(esbuildNative as unknown as EsbuildApi, input);
    const wasm = await bundle(esbuildWasm as unknown as EsbuildApi, input);
    expect(Buffer.from(wasm.js).equals(Buffer.from(native.js))).toBe(true);
    expect(native.css && wasm.css && Buffer.from(wasm.css).equals(Buffer.from(native.css))).toBe(
      true,
    );
  });

  it("the CLI flag set means the same as the JS-API options", async () => {
    const viaFlags = buildViaCliFlags();
    const viaApi = await bundle(esbuildNative as unknown as EsbuildApi, input);
    expect(viaFlags.js.equals(Buffer.from(viaApi.js))).toBe(true);
    expect(viaApi.css && viaFlags.css.equals(Buffer.from(viaApi.css))).toBe(true);
  });

  it("building twice is deterministic", async () => {
    const first = await bundle(esbuildNative as unknown as EsbuildApi, input);
    const second = await bundle(esbuildNative as unknown as EsbuildApi, input);
    expect(Buffer.from(first.js).equals(Buffer.from(second.js))).toBe(true);
  });

  it("a bare import without an alias fails with a clear message", async () => {
    const files = fixtureMap();
    files.set("src/main.ts", encoder.encode('import x from "react"; console.log(x);'));
    await expect(
      bundle(esbuildNative as unknown as EsbuildApi, { files, entry: "src/main.ts" }),
    ).rejects.toThrow(/bare import `react`/);
  });

  it("aliases resolve bare imports to in-map files", async () => {
    const files = fixtureMap();
    files.set("src/main.ts", encoder.encode('import { x } from "fake-lib"; console.log(x);'));
    files.set("stdlib/fake-lib.js", encoder.encode("export const x = 42;"));
    const out = await bundle(esbuildNative as unknown as EsbuildApi, {
      files,
      entry: "src/main.ts",
      aliases: { "fake-lib": "stdlib/fake-lib.js" },
    });
    expect(new TextDecoder().decode(out.js)).toContain("42");
  });
});
