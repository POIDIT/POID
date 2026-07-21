import { expect, type Page, test } from "@playwright/test";

/**
 * The Data Engine (M10) through the **real** sandbox → bridge → broker → SQL
 * engine path in Chromium: `poid.db.docs` CRUD, restart survival across a
 * fresh reader mount on the same IndexedDB-backed instance, and the 10k-row
 * query performance the DoD requires. The Node/vitest tier covers the engine
 * logic; this covers it running behind the postMessage boundary.
 */

declare global {
  interface Window {
    PoidHarness: {
      mount(args: { kind: string; sqlDb?: string; instanceId?: string }): void;
      run(): void;
      flushSql(): Promise<void>;
      probes(): { probe: string; [k: string]: unknown }[];
    };
  }
}

async function ready(page: Page): Promise<void> {
  await page.goto("/");
  await page.waitForFunction(() => typeof window.PoidHarness !== "undefined");
}

async function probe(page: Page, name: string, timeout = 20_000) {
  await page.waitForFunction((n) => window.PoidHarness.probes().some((p) => p.probe === n), name, {
    timeout,
  });
  return page.evaluate((n) => window.PoidHarness.probes().find((p) => p.probe === n), name);
}

test("poid.db.docs inserts and queries through the sandbox boundary", async ({ page }) => {
  await ready(page);
  const db = `docs-${Date.now()}`;
  await page.evaluate((sqlDb) => window.PoidHarness.mount({ kind: "docs", sqlDb }), db);
  await page.waitForSelector(".poid-consent__run");
  await page.evaluate(() => window.PoidHarness.run());

  const result = (await probe(page, "docs")) as { ok: boolean; total: number; seenBefore: number };
  expect(result.ok).toBe(true);
  expect(result.total).toBe(2);
  // A fresh database: the app saw nothing before it wrote.
  expect(result.seenBefore).toBe(0);
});

test("documents survive a reader restart (same instance, same store)", async ({ page }) => {
  await ready(page);
  const db = `docs-restart-${Date.now()}`;
  const instanceId = "kanban-instance";

  // First run: insert two cards, then flush the write-behind persist.
  await page.evaluate(
    ({ sqlDb, instanceId }) => window.PoidHarness.mount({ kind: "docs", sqlDb, instanceId }),
    { sqlDb: db, instanceId },
  );
  await page.waitForSelector(".poid-consent__run");
  await page.evaluate(() => window.PoidHarness.run());
  const first = (await probe(page, "docs")) as { total: number };
  expect(first.total).toBe(2);
  await page.evaluate(() => window.PoidHarness.flushSql());

  // Reload the page (a fresh reader) and mount the same instance on the same
  // IndexedDB store — the cards from the previous run must already be there.
  await ready(page);
  await page.evaluate(
    ({ sqlDb, instanceId }) => window.PoidHarness.mount({ kind: "docs", sqlDb, instanceId }),
    { sqlDb: db, instanceId },
  );
  await page.waitForSelector(".poid-consent__run");
  await page.evaluate(() => window.PoidHarness.run());
  const second = (await probe(page, "docs")) as { seenBefore: number; total: number };
  // It saw the two prior cards before writing, and now holds four.
  expect(second.seenBefore).toBe(2);
  expect(second.total).toBe(4);
});

test("10k-row indexed query returns in well under 50 ms", async ({ page }) => {
  await ready(page);
  await page.evaluate(() =>
    window.PoidHarness.mount({ kind: "perf", sqlDb: `perf-${Date.now()}` }),
  );
  await page.waitForSelector(".poid-consent__run");
  await page.evaluate(() => window.PoidHarness.run());

  const result = (await probe(page, "perf", 30_000)) as {
    ok: boolean;
    rows: number;
    total: number;
    ms: number;
  };
  expect(result.ok).toBe(true);
  expect(result.total).toBe(10_000);
  expect(result.rows).toBe(100);
  // The DoD bound. Generous headroom is fine; a regression into hundreds of
  // ms (e.g. a dropped index, a full scan) is what this guards against.
  expect(result.ms).toBeLessThan(50);
});
