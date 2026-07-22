/**
 * The first tests that have ever run against POID Studio itself.
 *
 * They are deliberately about the promises a user can see, not about internals:
 * a bare launch shows the hub, a document launch shows that document and never
 * the hub (UX rule 2), a good container runs after consent, and a container
 * carrying machine code is refused with an explanation instead of silence.
 */

import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import { fixture, HUB_PAGE, launchStudio, type StudioApp } from "./studio-app.js";

test.skip(
  process.platform !== "win32",
  "The desktop tier needs CDP, which only the Windows webview (WebView2) exposes.",
);

let app: StudioApp | undefined;

test.afterEach(async () => {
  await app?.close();
  app = undefined;
});

test("a bare launch opens the hub", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  await expect(hub.locator("#open-poid")).toHaveText(/Open a POID/);
  await expect(hub.locator("#conn-title")).toHaveText("Connections");
  // A throwaway app-data directory means a clean registry: proof that the
  // harness is not reading the maintainer's real connections.
  await expect(hub.locator("#conn-list .conn__row")).toHaveCount(0);
});

test("opening a document shows that document, and never the hub", async () => {
  app = await launchStudio({ open: fixture("valid/minimal-html.poid") });
  const reader = await app.reader();

  // Consent first: nothing runs before the user says so.
  const consent = reader.locator(".poid-consent");
  await expect(consent).toBeVisible();
  await expect(consent.locator(".poid-consent__run")).toHaveText("Run");

  // UX rule 2: a double-clicked file must not summon the Studio hub.
  const hubPages = app.browser
    .contexts()
    .flatMap((c) => c.pages())
    .filter((p) => HUB_PAGE.test(p.url()));
  expect(hubPages).toHaveLength(0);

  // Opening a POID *rewrites* it: the reader stamps `instance.id` into the
  // manifest, which is how a copy of a document becomes a document of its own
  // (SPEC §3.2, §6.3). Asserted here because it is genuinely surprising, and
  // because it is the reason this harness opens a copy rather than the
  // conformance fixture itself.
  const original = readFileSync(fixture("valid/minimal-html.poid"));
  const opened = app.documentPath ?? "";
  expect(opened).not.toBe("");
  await expect.poll(() => readFileSync(opened).equals(original)).toBe(false);
});

test("the application runs only after consent, in its own sandboxed frame", async () => {
  app = await launchStudio({ open: fixture("valid/minimal-html.poid") });
  const reader = await app.reader();

  // Before Run there is no application frame at all — the container is not
  // merely hidden, it has not been served.
  expect(reader.frames().filter((f) => f !== reader.mainFrame())).toHaveLength(0);

  await reader.locator(".poid-consent__run").click();

  const frame = await app.appFrame(reader);
  await expect(frame.locator("body")).not.toBeEmpty();
  // The chrome names the document and its storage mode once it is running.
  await expect(reader.locator(".poid-chrome__title")).not.toBeEmpty();
});

test("a container carrying machine code is refused, visibly and safely", async () => {
  app = await launchStudio({ open: fixture("invalid/contains-exe.poid") });
  const reader = await app.reader();

  const outcome = reader.locator(".poid-outcome");
  await expect(outcome).toBeVisible();
  await expect(outcome.locator(".poid-outcome__code")).toHaveText(/POID-\d+/);
  await expect(outcome.locator(".poid-outcome__safe")).toHaveText("Nothing in it was run.");

  // There is no consent button to press and nothing was served: refusal is not
  // a warning the user can click past.
  await expect(reader.locator(".poid-consent__run")).toHaveCount(0);
  expect(reader.frames().filter((f) => f !== reader.mainFrame())).toHaveLength(0);
});
