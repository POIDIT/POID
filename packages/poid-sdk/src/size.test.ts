import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";
import { describe, expect, it } from "vitest";

/**
 * The SDK ships inside every POID's runtime context, so it must stay tiny
 * (CONVENTIONS.md: `@poid/sdk` < 10 KB minified). This bundles the sandbox
 * bootstrap and asserts the byte budget. esbuild is a dev dependency; its JS
 * API locates its own platform binary.
 */
describe("bundle size budget", () => {
  it("the sandbox bootstrap minifies to under 10 KB", async () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const entry = join(here, "bootstrap.ts");
    const result = await esbuild.build({
      entryPoints: [entry],
      bundle: true,
      minify: true,
      format: "iife",
      platform: "browser",
      write: false,
      logLevel: "error",
    });
    const output = result.outputFiles[0];
    const bytes = output ? output.contents.length : Number.POSITIVE_INFINITY;
    expect(bytes, `bootstrap bundle is ${bytes} bytes`).toBeLessThan(10 * 1024);
  });
});
