/**
 * The M06 Python DoD, proven in a real browser: a pandas + matplotlib
 * script runs from a container's `deps/` wheels — engine verified before
 * use, wheels installed from local files, **zero contact with PyPI or any
 * CDN** (every request in the run is same-origin: the reader's own engine
 * assets and the container's wheels).
 *
 * Skips itself when the engine cache is absent — run
 * `node scripts/fetch-pyodide.mjs` and
 * `node scripts/fetch-wheels.mjs packages/poid-host/e2e/fixtures/python-chart`
 * first (CI does, with a cache).
 */

import { existsSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..");
const engineDir = join(repoRoot, "engines", ".cache", "pyodide");
const fixtureDeps = join(here, "fixtures", "python-chart", "deps");

const ready = existsSync(join(engineDir, "pyodide.js")) && existsSync(fixtureDeps);

test.describe("python profile (web+python)", () => {
  test.skip(!ready, "engine or wheels not fetched (see file header)");
  // Pyodide + numpy + pandas take a while to boot in CI.
  test.setTimeout(180_000);

  test("pandas + matplotlib run offline from deps/ wheels", async ({ page }) => {
    const wheels = readdirSync(fixtureDeps).filter((f) => f.endsWith(".whl"));
    expect(wheels.length).toBeGreaterThan(0);

    const offOrigin: string[] = [];
    page.on("request", (req) => {
      if (!req.url().startsWith("http://localhost")) offOrigin.push(req.url());
    });

    await page.goto("/");
    const result = await page.evaluate(
      async ({ wheelNames }) => {
        // 1. Verify EVERY engine file against the pinned manifest before
        //    executing anything (SPEC 5.4). Same code path as the reader.
        const manifest = await (await fetch("/engine-manifest.json")).json();
        for (const [file, expected] of Object.entries<string>(manifest.files)) {
          const bytes = new Uint8Array(await (await fetch(`/engine/${file}`)).arrayBuffer());
          const digest = await crypto.subtle.digest("SHA-256", bytes);
          const hex = [...new Uint8Array(digest)]
            .map((b) => b.toString(16).padStart(2, "0"))
            .join("");
          if (hex !== expected) throw new Error(`engine integrity: ${file}`);
        }

        // 2. Boot the verified engine from the reader's own assets.
        await new Promise<void>((resolve, reject) => {
          const s = document.createElement("script");
          s.src = "/engine/pyodide.js";
          s.onload = () => resolve();
          s.onerror = () => reject(new Error("engine load failed"));
          document.head.append(s);
        });
        const pyodide = await (
          window as unknown as {
            loadPyodide(o: { indexURL: string }): Promise<{
              unpackArchive(b: ArrayBuffer, fmt: string): void;
              runPythonAsync(code: string): Promise<unknown>;
            }>;
          }
        ).loadPyodide({ indexURL: "/engine/" });

        // 3. Install the container's wheels — from files, no pip, no PyPI.
        for (const name of wheelNames) {
          const bytes = await (await fetch(`/python-fixture/deps/${name}`)).arrayBuffer();
          pyodide.unpackArchive(bytes, "wheel");
        }

        // 4. Run the application source from the container.
        const code = await (await fetch("/python-fixture/main.py")).text();
        await pyodide.runPythonAsync(code);
        const json = await pyodide.runPythonAsync("import json; json.dumps(RESULT)");
        return JSON.parse(String(json)) as {
          total: number;
          png_prefix: string;
          png_len: number;
        };
      },
      { wheelNames: wheels },
    );

    expect(result.total).toBe(15);
    // A base64 PNG always starts with the PNG magic.
    expect(result.png_prefix).toBe("iVBORw0K");
    expect(result.png_len).toBeGreaterThan(1000);

    expect(offOrigin, `network left the machine: ${offOrigin.join(", ")}`).toEqual([]);
  });
});
