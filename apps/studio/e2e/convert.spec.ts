/**
 * The Converter panel (M12.2 / M12.2b), driven against the real Studio.
 *
 * The conversion IPC is the part that had never run in the product: the shared
 * poid-convert pipeline, invoked from a real Studio window, turning bytes into
 * a `.poid`. The build path adds the second half — esbuild-wasm compiling and
 * bundling inside WebView2 — which these tests exercise through the real file
 * input, so the engine actually runs where it will in production.
 *
 * The final save step ends in a native dialog that cannot be automated
 * headlessly; the tests assert the state reached just before it (a built,
 * packed POID) rather than the save itself.
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

test("a single HTML file converts to a .poid with no build", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const prep = (await invoke(hub, "convert_prepare", {
    files: [
      { rel: "page.html", bytesBase64: B64("<!doctype html><title>Hi</title><h1>hello</h1>") },
    ],
    displayName: "Hi",
  })) as { needsBuild: boolean; poidBase64: string | null; kind: string };

  expect(prep.needsBuild).toBe(false);
  expect(prep.kind).toBe("html");
  // A .poid is a ZIP: its first bytes are the local-file-header magic "PK".
  const head = Buffer.from(prep.poidBase64 ?? "", "base64")
    .subarray(0, 2)
    .toString("latin1");
  expect(head).toBe("PK");
});

test("a JSX project comes back as a build request with react staged", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const prep = (await invoke(hub, "convert_prepare", {
    files: [
      {
        rel: "main.jsx",
        bytesBase64: B64('import React from "react"; export default () => <h1>hi</h1>;'),
      },
    ],
    displayName: "app",
  })) as {
    needsBuild: boolean;
    token: string | null;
    entry: string | null;
    aliases: Record<string, string>;
  };

  expect(prep.needsBuild).toBe(true);
  expect(prep.token).toBeTruthy();
  expect(prep.entry).toBeTruthy();
  // The Standard Library resolved react for the automatic JSX runtime.
  expect(Object.keys(prep.aliases)).toContain("react");
});

test("the build engine is available to the converter", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const status = (await invoke(hub, "esbuild_engine_status", {})) as {
    installed: boolean;
    version: string;
  };
  test.skip(!status.installed, "no staged esbuild engine (run scripts/fetch-esbuild.mjs)");
  expect(status.version).toBe("0.25.12");
});

test("a JSX component builds end-to-end with esbuild-wasm in the webview", async () => {
  app = await launchStudio();
  const hub = await app.hub();

  const status = (await invoke(hub, "esbuild_engine_status", {})) as { installed: boolean };
  test.skip(!status.installed, "no staged esbuild engine (run scripts/fetch-esbuild.mjs)");

  await hub.locator('.hub-nav__item[href="#convert"]').click();
  await expect(hub.locator(".convert__drop")).toBeVisible();

  // Feed a real .jsx through the panel's file input — the same path a user's
  // pick takes, so esbuild-wasm runs in WebView2 exactly as it will in
  // production. (Drag-drop and the folder picker cannot be automated; a plain
  // file input can.)
  const dir = mkdtempSync(join(tmpdir(), "poid-convert-jsx-"));
  const jsx = join(dir, "App.jsx");
  writeFileSync(jsx, "export default function App() { return <h1>built</h1>; }\n");

  try {
    await hub.locator("#convert-file-input").setInputFiles(jsx);

    // The status walks: engine ready → Building… → Packing…. Reaching Packing
    // proves esbuild-wasm bundled the component and convert_finish assembled
    // the .poid — everything up to the native save dialog, which we do not
    // drive. A build error would surface here instead, failing the poll.
    await expect(hub.locator(".convert__status")).toHaveText(/Packing…|Saved|Ready to save/, {
      timeout: 90_000,
    });
    await expect(hub.locator(".convert__status--error")).toHaveCount(0);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
