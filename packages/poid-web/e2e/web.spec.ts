/**
 * The Web Reader against the conformance suite, in a real Chromium:
 *
 * - every `spec/conformance/valid/` file opens to the correct screen,
 * - every `spec/conformance/invalid/` file is rejected with the registry
 *   code its `.expected.json` demands — the same hostile files the desktop
 *   tier blocks (M04) are blocked here,
 * - the M05 Definition of Done is asserted mechanically: zero network
 *   requests after the initial page load, and full function offline.
 */

import { readdirSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateRawSync } from "node:zlib";
import { expect, type Page, test } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
const conformance = join(here, "..", "..", "..", "spec", "conformance");

function fixtures(kind: "valid" | "invalid"): { name: string; poid: string; code?: string }[] {
  const dir = join(conformance, kind);
  return readdirSync(dir)
    .filter((f) => f.endsWith(".poid"))
    .map((f) => {
      const expected = JSON.parse(
        readFileSync(join(dir, `${basename(f, ".poid")}.expected.json`), "utf8"),
      ) as { code?: string };
      const entry: { name: string; poid: string; code?: string } = {
        name: basename(f, ".poid"),
        poid: join(dir, f),
      };
      if (expected.code) entry.code = expected.code;
      return entry;
    });
}

async function openFixture(page: Page, poidPath: string): Promise<void> {
  await page.setInputFiles("#file-input", poidPath);
}

/** Reads one entry from a ZIP by walking local file headers (poid-core packs
 * without data descriptors, so sizes are in the headers). */
function readZipEntry(bytes: Buffer, name: string): Buffer {
  let off = 0;
  while (off + 30 <= bytes.length && bytes.readUInt32LE(off) === 0x04034b50) {
    const method = bytes.readUInt16LE(off + 8);
    const compSize = bytes.readUInt32LE(off + 18);
    const nameLen = bytes.readUInt16LE(off + 26);
    const extraLen = bytes.readUInt16LE(off + 28);
    const entryName = bytes.subarray(off + 30, off + 30 + nameLen).toString("utf8");
    const dataStart = off + 30 + nameLen + extraLen;
    const data = bytes.subarray(dataStart, dataStart + compSize);
    if (entryName === name) return method === 8 ? inflateRawSync(data) : Buffer.from(data);
    off = dataStart + compSize;
  }
  throw new Error(`entry ${name} not found in the downloaded container`);
}

/** Valid `type: app`, `web`-profile fixtures and how their consent screen is titled. */
const RUNNABLE: Record<string, string> = {
  "minimal-html": "Minimal HTML",
  "react-app": "React App",
  "embedded-data": "Embedded Data",
  "vault-mode": "Vault Mode",
  slots: "Slots",
  protected: "Protected",
  signed: "Signed",
  "unknown-manifest-fields": "Forward Compatible",
  "large-but-legal": "Large But Legal",
};

test.beforeEach(async ({ page }) => {
  await page.goto("/");
});

test.describe("valid fixtures", () => {
  for (const fixture of fixtures("valid")) {
    test(`opens ${fixture.name} to the right screen`, async ({ page }) => {
      await openFixture(page, fixture.poid);

      if (fixture.name === "python-pyodide") {
        await expect(page.locator("#outcome")).toContainText("needs an engine");
        await expect(page.locator("#outcome")).toContainText("pyodide");
      } else if (fixture.name === "workspace") {
        await expect(page.locator("#outcome")).toContainText("workspace");
        await expect(page.locator("#outcome")).toContainText("POID Studio");
      } else if (fixture.name === "data-container") {
        await expect(page.locator("#outcome")).toContainText("data container");
        await expect(page.locator("#outcome")).toContainText("org.poid.conformance.survey");
      } else {
        // Runnable: the consent screen appears and execution has NOT begun.
        await expect(page.locator(".poid-consent")).toBeVisible();
        await expect(page.locator("iframe")).toHaveCount(0);
        const title = RUNNABLE[fixture.name];
        if (title) await expect(page.locator(".poid-consent__name")).toHaveText(title);
      }
    });
  }

  test("Run mounts the sandbox and the app actually executes", async ({ page }) => {
    await openFixture(page, join(conformance, "valid", "minimal-html.poid"));
    await page.locator(".poid-consent__run").click();
    await expect(page.locator(".poid-chrome__title")).toHaveText("Minimal HTML");
    const frame = page.frameLocator("#reader-host iframe");
    await expect(frame.locator("h1")).toHaveText("ok");
    // The sandbox attribute is the one line that must never regress.
    await expect(page.locator("#reader-host iframe")).toHaveAttribute("sandbox", "allow-scripts");
  });

  test("Cancel never executes the application", async ({ page }) => {
    await openFixture(page, join(conformance, "valid", "minimal-html.poid"));
    await page.locator(".poid-consent__cancel").click();
    await expect(page.locator("iframe")).toHaveCount(0);
    await expect(page.locator("#home")).toBeVisible();
  });

  test("a signed container shows the verified publisher", async ({ page }) => {
    await openFixture(page, join(conformance, "valid", "signed.poid"));
    await expect(page.locator(".poid-consent__sig")).toContainText("Verified publisher");
  });

  test("embedded data seeds the engine and travels in the downloaded file", async ({ page }) => {
    await openFixture(page, join(conformance, "valid", "embedded-data.poid"));
    await page.locator(".poid-consent__run").click();
    await expect(page.locator(".poid-chrome__title")).toHaveText("Embedded Data");

    const downloadPromise = page.waitForEvent("download");
    await page.locator(".poid-toolbar__download").click();
    const download = await downloadPromise;
    expect(download.suggestedFilename()).toBe("embedded-data.poid");

    const path = await download.path();
    const bytes = readFileSync(path);
    // A conformant container: `mimetype` first (readable in the raw ZIP)...
    expect(bytes.subarray(0, 100).toString("latin1")).toContain("application/vnd.poid+zip");
    // ...and the state store carrying the seeded content, human-readable
    // (SPEC §6.2) once decompressed.
    const store = JSON.parse(readZipEntry(bytes, "data/store.json").toString("utf8")) as {
      todos?: { text: string }[];
    };
    expect(store.todos?.[0]?.text).toBe("ship the conformance suite");
  });

  test("vault mode shows no download button", async ({ page }) => {
    await openFixture(page, join(conformance, "valid", "vault-mode.poid"));
    await page.locator(".poid-consent__run").click();
    await expect(page.locator(".poid-chrome__badge")).toHaveText("vault");
    await expect(page.locator(".poid-toolbar__download")).toHaveCount(0);
  });
});

test.describe("hostile fixtures", () => {
  for (const fixture of fixtures("invalid")) {
    test(`rejects ${fixture.name} with ${fixture.code}`, async ({ page }) => {
      await openFixture(page, fixture.poid);
      const outcome = page.locator("#outcome");
      await expect(outcome).toContainText(fixture.code ?? "POID-");
      await expect(outcome).toContainText("Nothing from this file was executed.");
      await expect(page.locator("iframe")).toHaveCount(0);
    });
  }
});

test.describe("definition of done", () => {
  test("zero network requests after the initial page load", async ({ page }) => {
    await page.goto("/");
    // Let the shell settle completely: scripts, the WASM core, the worker.
    await page.waitForLoadState("networkidle");

    const requests: string[] = [];
    page.on("request", (req) => {
      // blob: URLs are in-page object URLs (the sandbox iframe, the download);
      // they never leave the browser. Network means http(s).
      if (req.url().startsWith("http")) requests.push(req.url());
    });

    // A full user journey: validate, consent, run, mutate nothing, download.
    await openFixture(page, join(conformance, "valid", "embedded-data.poid"));
    await page.locator(".poid-consent__run").click();
    await expect(page.locator(".poid-chrome__title")).toHaveText("Embedded Data");
    const downloadPromise = page.waitForEvent("download");
    await page.locator(".poid-toolbar__download").click();
    await downloadPromise;
    // And a hostile file for good measure.
    await page.locator(".poid-toolbar__close").click();
    await openFixture(page, join(conformance, "invalid", "contains-exe.poid"));
    await expect(page.locator("#outcome")).toContainText("POID-020");

    expect(requests, `unexpected network traffic: ${requests.join(", ")}`).toEqual([]);
  });

  test("fully functional offline after first load", async ({ page, context }) => {
    await page.goto("/");
    await page.evaluate(() => navigator.serviceWorker.ready);
    await page.waitForLoadState("networkidle");

    // The build stamped a real digest into the cache name — a lingering
    // `__BUILD_HASH__` placeholder would freeze the shell cache forever.
    const cacheNames = await page.evaluate(() => caches.keys());
    expect(cacheNames).toEqual([expect.stringMatching(/^poid-web-[0-9a-f]{16}$/)]);

    await context.setOffline(true);
    try {
      await page.reload();
      await expect(page.locator("#drop-zone")).toBeVisible();
      await openFixture(page, join(conformance, "valid", "minimal-html.poid"));
      await page.locator(".poid-consent__run").click();
      await expect(page.frameLocator("#reader-host iframe").locator("h1")).toHaveText("ok");
    } finally {
      await context.setOffline(false);
    }
  });
});
