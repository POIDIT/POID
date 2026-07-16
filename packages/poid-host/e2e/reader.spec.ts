import { expect, type Page, test } from "@playwright/test";

/**
 * The M04 Definition of Done, proven against real Chromium. Each hostile POID
 * attempts one attack; the browser must defeat it.
 */

declare global {
  interface Window {
    PoidHarness: {
      mount(args: {
        kind: string;
        watchdog?: { intervalMs: number; timeoutMs: number };
      }): Promise<void>;
      run(): void;
      cancel(): void;
      stop(): void;
      probes(): { probe: string; [k: string]: unknown }[];
      hasConsent(): boolean;
      hasIframe(): boolean;
      iframeSandbox(): string | null;
      isUnresponsive(): boolean;
    };
  }
}

async function ready(page: Page): Promise<void> {
  await page.goto("/");
  await page.waitForFunction(() => typeof window.PoidHarness !== "undefined");
}

async function firstProbe(page: Page, name: string) {
  await page.waitForFunction((n) => window.PoidHarness.probes().some((p) => p.probe === n), name, {
    timeout: 10_000,
  });
  return page.evaluate((n) => window.PoidHarness.probes().find((p) => p.probe === n), name);
}

test("consent is shown first; the app does not run until Run is clicked", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "good" }));

  // Consent is present; no iframe exists yet, so the container cannot suppress
  // or race the consent screen — it is not even loaded.
  expect(await page.evaluate(() => window.PoidHarness.hasConsent())).toBe(true);
  expect(await page.evaluate(() => window.PoidHarness.hasIframe())).toBe(false);

  await page.evaluate(() => window.PoidHarness.run());
  expect(await page.evaluate(() => window.PoidHarness.hasIframe())).toBe(true);
});

test("the sandbox is allow-scripts and never allow-same-origin", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "good" }));
  await page.evaluate(() => window.PoidHarness.run());
  const sandbox = await page.evaluate(() => window.PoidHarness.iframeSandbox());
  expect(sandbox).toBe("allow-scripts");
  expect(sandbox).not.toContain("allow-same-origin");
});

test("fetch() to any origin is blocked by CSP", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "fetch" }));
  await page.evaluate(() => window.PoidHarness.run());
  const probe = await firstProbe(page, "fetch");
  expect(probe).toMatchObject({ blocked: true });
});

test("the app cannot escape the iframe to reach the host", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "escape" }));
  await page.evaluate(() => window.PoidHarness.run());
  const probe = await firstProbe(page, "escape");
  expect(probe).toMatchObject({ blocked: true });
});

test("the watchdog terminates an app that hangs its event loop", async ({ page }) => {
  await ready(page);
  await page.evaluate(() =>
    window.PoidHarness.mount({ kind: "loop", watchdog: { intervalMs: 100, timeoutMs: 500 } }),
  );
  await page.evaluate(() => window.PoidHarness.run());
  // The app starts, then blocks; the watchdog detects the missed pongs.
  await firstProbe(page, "loop-start");
  await page.waitForFunction(() => window.PoidHarness.isUnresponsive(), undefined, {
    timeout: 10_000,
  });
  // Stop always works, even against a hung app.
  await page.evaluate(() => window.PoidHarness.stop());
  expect(await page.evaluate(() => window.PoidHarness.hasIframe())).toBe(false);
});

test("the full bridge works in a real browser (kv round-trip)", async ({ page }) => {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "good" }));
  await page.evaluate(() => window.PoidHarness.run());
  const probe = await firstProbe(page, "kv");
  expect(probe).toMatchObject({ ok: true, value: "hello" });
});
