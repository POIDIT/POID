/**
 * The hub shell: navigation between panels.
 *
 * These assertions are what lets the rest of M12 be added as panels without
 * re-testing the shell each time — a new tool is one entry in `PANELS`, and
 * its own spec can jump straight to it by hash.
 */

import { expect, test } from "@playwright/test";
import { launchStudio, type StudioApp } from "./studio-app.js";

test.skip(
  process.platform !== "win32",
  "The desktop tier needs CDP, which only the Windows webview (WebView2) exposes.",
);

let app: StudioApp | undefined;

test.afterEach(async () => {
  await app?.close();
  app = undefined;
});

test("a bare launch opens the hub on its first panel", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  await expect(hub.locator("#panel-title")).toHaveText("Open a POID");
  await expect(hub.locator("#open-poid")).toBeVisible();
  await expect(hub.locator('.hub-nav__item[href="#home"]')).toHaveAttribute("aria-current", "page");
});

test("choosing a tool swaps the panel, in the same window", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  await hub.locator('.hub-nav__item[href="#connections"]').click();

  await expect(hub.locator("#panel-title")).toHaveText("Connections");
  // The previous panel is gone, not merely hidden: each panel owns its markup.
  await expect(hub.locator("#open-poid")).toHaveCount(0);
  // A throwaway app-data directory means a clean registry — proof that the
  // harness is not reading the maintainer's real connections.
  await expect(hub.locator("#conn-list .conn__row")).toHaveCount(0);
  await expect(hub.locator(".conn__empty")).toBeVisible();

  // One window throughout. Switching tools must never spawn a second.
  expect(app.browser.contexts().flatMap((c) => c.pages())).toHaveLength(1);
});

test("a panel can be reached directly by its hash", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  await hub.evaluate(() => {
    window.location.hash = "#connections";
  });
  await expect(hub.locator("#panel-title")).toHaveText("Connections");

  // A hash naming nothing falls back to the first panel rather than showing an
  // empty window.
  await hub.evaluate(() => {
    window.location.hash = "#nonsense";
  });
  await expect(hub.locator("#panel-title")).toHaveText("Open a POID");
});
