import { expect, type Page, test } from "@playwright/test";

/**
 * The M11 Definition of Done, proven against real Chromium:
 *
 * > A malicious POID attempting to read a Connection secret fails, and the
 * > failure is a test.
 *
 * The backend behind `poid.db.sql` is rigged to fail the way a real driver
 * does — by quoting the connection string it was given. That is the realistic
 * shape of this leak, and it is the one the boundary has to stop. The
 * application then sweeps every route the credential could take out of the
 * sandbox, hands back everything it found, and this spec searches it.
 */

declare global {
  interface Window {
    PoidHarness: {
      mount(args: { kind: string }): void;
      run(): void;
      probes(): { probe: string; [k: string]: unknown }[];
      connectionSecret(): string;
      diagnostics(): { method: string | null; code: string; detail: string }[];
    };
  }
}

async function ready(page: Page): Promise<void> {
  await page.goto("/");
  await page.waitForFunction(() => typeof window.PoidHarness !== "undefined");
}

async function runThief(page: Page) {
  await ready(page);
  await page.evaluate(() => window.PoidHarness.mount({ kind: "thief" }));
  await page.waitForFunction(() => document.querySelector(".poid-consent") !== null);
  await page.evaluate(() => window.PoidHarness.run());
  await page.waitForFunction(
    () => window.PoidHarness.probes().some((p) => p.probe === "thief"),
    undefined,
    { timeout: 15_000 },
  );
  return page.evaluate(() => ({
    probe: window.PoidHarness.probes().find((p) => p.probe === "thief") as {
      harvest: string;
      sqlError: { name: string; code: string; message: string } | null;
    },
    secret: window.PoidHarness.connectionSecret(),
    diagnostics: window.PoidHarness.diagnostics(),
    hostHtml: document.documentElement.outerHTML,
  }));
}

test("a POID that hunts for the connection credential finds nothing", async ({ page }) => {
  const { probe, secret } = await runThief(page);

  // Everything the application could reach: app.info(), the failed query's
  // error object and stack, the injected bootstrap, the global names, the
  // shape of `poid`, and its own served HTML.
  expect(probe.harvest.length).toBeGreaterThan(100); // it really did look
  expect(probe.harvest).not.toContain(secret);
  expect(probe.harvest).not.toContain("sup3rs3cret-pw");
  // Not even the pieces: host and database are as much a disclosure as the
  // password when the app is not supposed to know which backend answered.
  expect(probe.harvest).not.toContain("db.example.com");
});

test("the failed query returns a code and a constant message, not the driver's text", async ({
  page,
}) => {
  const { probe, secret } = await runThief(page);

  expect(probe.sqlError).not.toBeNull();
  // The code is the contract (RUNTIME-API §9); the reason is not.
  expect(probe.sqlError?.code).toBe("INTERNAL");
  expect(probe.sqlError?.message).not.toContain(secret);
  expect(probe.sqlError?.message).not.toContain("password authentication failed");
  expect(probe.sqlError?.message).toBe("something went wrong inside the reader");
});

test("the credential does not sit in the host context either", async ({ page }) => {
  const { secret, hostHtml } = await runThief(page);

  // SPEC §7.1 as amended in M11: not only the sandbox — no browser-engine
  // context, including the reader's own. Through M10 the session object here
  // held the secret, and this assertion would have failed.
  expect(hostHtml).not.toContain(secret);

  const reachable = await page.evaluate((needle) => {
    const found: string[] = [];
    for (const key of Object.getOwnPropertyNames(window)) {
      try {
        const value = (window as unknown as Record<string, unknown>)[key];
        if (typeof value === "string" && value.includes(needle)) found.push(key);
        else if (value && typeof value === "object" && JSON.stringify(value)?.includes(needle)) {
          found.push(key);
        }
      } catch {
        // Cross-origin or throwing accessor: nothing readable here either.
      }
    }
    return found;
  }, secret);
  expect(reachable).toEqual([]);
});

test("the reason survives — for the user, on the host side", async ({ page }) => {
  const { diagnostics, secret } = await runThief(page);

  // Scrubbing the boundary must not mean throwing the diagnosis away: a person
  // debugging their own database is entitled to the real message. It just
  // reaches them through the host, not through the application (SPEC §7.2.4).
  const sqlFailure = diagnostics.find((d) => d.method === "db.sql.exec");
  expect(sqlFailure).toBeDefined();
  expect(sqlFailure?.detail).toContain("password authentication failed");

  // Studio redacts known credentials before this reaches the user's log
  // (poid_broker::Redactor); the raw value is what the host layer receives.
  expect(sqlFailure?.detail).toContain(secret);
});
