/**
 * The Converter panel (M12.2), driven against the real Studio.
 *
 * The drop → save flow ends in a native save dialog, which cannot be
 * automated headlessly. What these tests prove is the part that matters and
 * had never run in the product: the conversion IPC itself — the shared
 * `poid-convert` pipeline, invoked from a real Studio window over real Tauri
 * IPC, turning bytes into a `.poid` (or honestly declining a project that
 * needs a build it cannot yet run).
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

/** base64 of a UTF-8 string, in the page. */
const B64 = (text: string) => Buffer.from(text, "utf8").toString("base64");

/** Invokes a Studio IPC command from inside the hub window. */
async function invoke(
  page: import("@playwright/test").Page,
  cmd: string,
  args: unknown,
): Promise<unknown> {
  return page.evaluate(
    ([c, a]) =>
      (
        window as unknown as {
          __TAURI_INTERNALS__: { invoke(cmd: string, args: unknown): Promise<unknown> };
        }
      ).__TAURI_INTERNALS__.invoke(c as string, a),
    [cmd, args] as const,
  );
}

test("the Convert panel is a tool in the hub", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  await hub.locator('.hub-nav__item[href="#convert"]').click();
  await expect(hub.locator("#panel-title")).toHaveText("Convert to a POID");
  await expect(hub.locator(".convert__drop")).toBeVisible();
});

test("a single HTML file converts to a .poid", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const outcome = (await invoke(hub, "convert_to_poid", {
    files: [
      { rel: "page.html", bytesBase64: B64("<!doctype html><title>Hi</title><h1>hello</h1>") },
    ],
    displayName: "Hi",
  })) as { needsBuild: boolean; poidBase64: string | null; kind: string };

  expect(outcome.needsBuild).toBe(false);
  expect(outcome.kind).toBe("html");
  expect(outcome.poidBase64).toBeTruthy();
  // A .poid is a ZIP: its first bytes are the local-file-header magic "PK".
  const head = Buffer.from(outcome.poidBase64 ?? "", "base64")
    .subarray(0, 2)
    .toString("latin1");
  expect(head).toBe("PK");
});

test("a static folder keeps its files and converts", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const outcome = (await invoke(hub, "convert_to_poid", {
    files: [
      {
        rel: "index.html",
        bytesBase64: B64("<!doctype html><link rel=stylesheet href=s.css><h1>hi</h1>"),
      },
      { rel: "s.css", bytesBase64: B64("h1{color:red}") },
    ],
    displayName: "site",
  })) as { needsBuild: boolean; fileCount: number };

  expect(outcome.needsBuild).toBe(false);
  // index.html plus the stylesheet — a static site is packed as-is.
  expect(outcome.fileCount).toBeGreaterThanOrEqual(2);
});

test("a project that needs building is declined honestly, not half-converted", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const outcome = (await invoke(hub, "convert_to_poid", {
    files: [
      {
        rel: "main.jsx",
        bytesBase64: B64('import React from "react"; export default () => <h1>hi</h1>;'),
      },
    ],
    displayName: "app",
  })) as { needsBuild: boolean; poidBase64: string | null; message: string | null };

  expect(outcome.needsBuild).toBe(true);
  expect(outcome.poidBase64).toBeNull();
  expect(outcome.message).toContain("build");
});
