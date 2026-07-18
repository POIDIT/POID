/**
 * Sanity coverage for the catalog map itself. The end-to-end proof — real
 * bundles resolved and built offline — lives in
 * `packages/poid-toolchain/src/stdlib.test.ts`, which needs the built
 * library on disk; this suite needs only the committed JSON.
 */

import { describe, expect, it } from "vitest";
import { exclusionReason, resolveStdlib, STDLIB_ESBUILD, stdlibSpecifiers } from "./index.js";

describe("stdlib catalog", () => {
  it("is pinned to the build contract's esbuild version", () => {
    expect(STDLIB_ESBUILD).toBe("0.25.12");
  });

  it("exposes every specifier with a version and a bundle path", () => {
    const specs = stdlibSpecifiers();
    expect(specs.size).toBeGreaterThan(20);
    const react = specs.get("react");
    expect(react).toMatchObject({ pkg: "react", file: "react.js" });
    expect(react?.sha256).toMatch(/^[0-9a-f]{64}$/);
  });

  it("resolves known specifiers and reports unknown ones", () => {
    const { selection, missing } = resolveStdlib(["clsx", "not-a-real-package"]);
    expect(selection.has("clsx")).toBe(true);
    expect(missing).toEqual(["not-a-real-package"]);
  });

  it("names curated exclusions", () => {
    expect(exclusionReason("svelte")).toBeTruthy();
    expect(exclusionReason("clsx")).toBeUndefined();
  });
});
