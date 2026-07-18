/**
 * The Standard Library in action (ARCHITECTURE §5.2, Tier 1): a React
 * counter — the exact shape AI chatbots emit — bundles through the in-app
 * engine with catalog aliases, offline, and both engines agree byte-for-byte.
 *
 * Requires the built library (`pnpm --filter @poid/stdlib build:lib`);
 * the suite fails with a pointer when it is missing.
 */

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { bundleRelPath, exclusionReason, resolveStdlib } from "@poid/stdlib";
import * as esbuildNative from "esbuild";
import * as esbuildWasm from "esbuild-wasm";
import { describe, expect, it } from "vitest";
import { type BundleInput, bundle, type EsbuildApi } from "./engine.js";

const here = dirname(fileURLToPath(import.meta.url));
const libDir = join(here, "..", "..", "poid-stdlib", "lib");

const encoder = new TextEncoder();

const COUNTER_TSX = `
import { useState } from "react";
import { createRoot } from "react-dom/client";

function Counter() {
  const [count, setCount] = useState(0);
  return (
    <button onClick={() => setCount((c) => c + 1)}>count is {count}</button>
  );
}

const root = document.createElement("div");
document.body.append(root);
createRoot(root).render(<Counter />);
`;

/** Builds the BundleInput for a source file plus the stdlib selection it
 * needs, loading catalog bundles from disk the way Studio loads them from
 * its resources. */
function withStdlib(source: string, requested: string[]): BundleInput {
  const { selection, missing } = resolveStdlib(requested);
  expect(missing).toEqual([]);
  const files = new Map<string, Uint8Array>([["src/main.tsx", encoder.encode(source)]]);
  const aliases: Record<string, string> = {};
  for (const [specifier, spec] of selection) {
    const diskPath = join(libDir, spec.file);
    expect(
      existsSync(diskPath),
      `missing ${diskPath} — run: pnpm --filter @poid/stdlib build:lib`,
    ).toBe(true);
    const mapPath = `stdlib/${bundleRelPath(specifier)}`;
    files.set(mapPath, readFileSync(diskPath));
    aliases[specifier] = mapPath;
  }
  return { files, entry: "src/main.tsx", aliases };
}

describe("standard library", () => {
  // JSX with the automatic runtime injects `react/jsx-runtime` even though
  // the source never names it — the converter always requests it for JSX
  // sources.
  const REQUESTED = ["react", "react/jsx-runtime", "react-dom/client"];

  it("bundles a React counter offline, identically on both engines", async () => {
    const input = withStdlib(COUNTER_TSX, REQUESTED);
    const native = await bundle(esbuildNative as unknown as EsbuildApi, input);
    const wasm = await bundle(esbuildWasm as unknown as EsbuildApi, input);
    expect(Buffer.from(native.js).equals(Buffer.from(wasm.js))).toBe(true);
    // The whole framework travels in the bundle; nothing resolves at open time.
    expect(native.js.length).toBeGreaterThan(100_000);
    const text = new TextDecoder().decode(native.js);
    expect(text).toContain("count is");
    expect(text).not.toContain('from"react"');
  });

  it("follows externals transitively (react-dom/client pulls react and scheduler)", () => {
    const { selection, missing } = resolveStdlib(["react-dom/client"]);
    expect(missing).toEqual([]);
    const names = [...selection.keys()].sort();
    expect(names).toEqual(["react", "react-dom", "react-dom/client", "scheduler"]);
  });

  it("answers unknown imports with a clear miss, never a fetch", () => {
    const { missing } = resolveStdlib(["tone", "left-pad"]);
    expect(missing).toEqual(["left-pad", "tone"]);
  });

  it("explains curated exclusions", () => {
    expect(exclusionReason("svelte")).toContain("compiler");
    expect(exclusionReason("tailwindcss/browser")).toContain("Converter");
    expect(exclusionReason("react")).toBeUndefined();
  });
});
