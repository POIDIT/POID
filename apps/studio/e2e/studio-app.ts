/**
 * Drives the **real** POID Studio: the built desktop binary, its own windows,
 * its own Rust backend.
 *
 * Every other test tier in this repository stops at the browser. `vitest`
 * covers logic, and the Playwright suites in `@poid/host` and `@poid/web` drive
 * the *web* reader in Chromium. That left the desktop product — the thing
 * people actually download — verified only by whatever the maintainer
 * remembered to click. `KNOWN-GAPS.md` called that the single most important
 * gap at the end of M11, and this file is the answer to it.
 *
 * How it works: Tauri's webview on Windows is WebView2, which is Chromium, so
 * it speaks the Chrome DevTools Protocol when asked. We launch the binary with
 * a debugging port, attach Playwright over CDP, and from there a Studio window
 * is an ordinary `Page` with ordinary selectors and auto-waiting.
 *
 * Three constraints are worth knowing before writing a test against this:
 *
 * 1. **Windows only.** macOS and Linux use WebKit, which has no CDP endpoint.
 *    Those platforms stay on the maintainer's own hands until someone teaches
 *    this harness the WebKit remote inspector protocol.
 * 2. **One Studio at a time.** The single-instance plugin forwards a second
 *    launch into the first process, so a second concurrent test would drive
 *    the first test's application. Hence `workers: 1`, and hence the running
 *    -instance check in `launchStudio`.
 * 3. **The maintainer's own data is never touched.** Tauri derives its app data
 *    directory from `APPDATA`, so pointing that at a temporary directory gives
 *    each launch an empty vault and an empty connection registry. No production
 *    code knows this harness exists.
 */

import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { type Browser, chromium, type Page } from "@playwright/test";

const here = dirname(fileURLToPath(import.meta.url));
/** Repository root, from `apps/studio/e2e`. */
const repoRoot = join(here, "..", "..", "..");

/** The debugging port handed to WebView2. Deliberately not 9222: that is the
 * port a developer's own Chrome or an ad-hoc CDP script is most likely to be
 * sitting on, and attaching to the wrong process fails in a very confusing
 * way. */
const CDP_PORT = Number(process.env.POID_STUDIO_CDP_PORT ?? 9333);

/**
 * The hub window and a Reader window, told apart by the page Tauri loaded.
 *
 * The hub matches the *root* as well as `index.html`: Tauri serves the
 * frontend's index at `/`, so the hub window's URL is bare
 * `http://tauri.localhost/` even though `windows.rs` asked for `index.html`.
 */
export const HUB_PAGE = /^\w+:\/\/[^/]+\/(?:index\.html)?(?:[?#]|$)/;
export const READER_PAGE = /\/reader\.html(?:[?#]|$)/;

/** How long a desktop application may take to show its first window. Generous:
 * a cold start on a CI runner pays for the webview process too. */
const LAUNCH_TIMEOUT_MS = 60_000;

/**
 * Locates the built Studio binary. This never *builds* it: a test that
 * silently triggers a fifteen-minute release build is a test nobody runs
 * twice. When it is missing, say exactly which command produces it.
 */
export function studioExecutable(): string {
  const override = process.env.POID_STUDIO_BIN;
  if (override) {
    if (!existsSync(override)) {
      throw new Error(`POID_STUDIO_BIN points at ${override}, which does not exist`);
    }
    return override;
  }

  const exe = process.platform === "win32" ? "poid-studio.exe" : "poid-studio";
  const target = join(repoRoot, "apps", "studio", "src-tauri", "target");
  // Release first: it is what CI builds and what a user installs. A debug
  // build is accepted so a developer iterating locally is not forced through
  // an optimised link every time.
  for (const profile of ["release", "debug"]) {
    const candidate = join(target, profile, exe);
    if (existsSync(candidate)) return candidate;
  }

  throw new Error(
    "No Studio binary found. Build one first:\n" +
      "  pnpm --filter @poid/studio build:ui\n" +
      "  cargo build --manifest-path apps/studio/src-tauri/Cargo.toml --release\n" +
      `(or set POID_STUDIO_BIN to a binary elsewhere; looked under ${target})`,
  );
}

/**
 * Refuses to run while a Studio is already up.
 *
 * Without this the single-instance plugin would hand our command line to the
 * maintainer's running application: the test would hang waiting for a window
 * that appeared somewhere else, and — worse — a document would open in their
 * session. Failing with a sentence they can act on is the whole feature.
 */
function assertNoRunningStudio(): void {
  if (process.platform !== "win32") return;
  let listing = "";
  try {
    listing = execFileSync("tasklist", ["/fi", "imagename eq poid-studio.exe", "/nh"], {
      encoding: "utf8",
    });
  } catch {
    // No tasklist, or it failed: proceed rather than block the suite on a
    // diagnostic that is not itself the thing under test.
    return;
  }
  if (listing.toLowerCase().includes("poid-studio.exe")) {
    throw new Error(
      "POID Studio is already running. Close every Studio and Reader window first — " +
        "the single-instance lock means a second launch would be forwarded into that " +
        "process instead of starting the one these tests drive.",
    );
  }
}

/** Options for one launch. */
export interface LaunchOptions {
  /**
   * A `.poid` to open, which puts Studio in Reader mode — the double-click
   * path. Omit it for a bare launch, which opens the hub.
   *
   * The file is **copied** into the launch's temporary directory and the copy
   * is what Studio opens; see `launchStudio` for why that matters.
   */
  open?: string;
}

/** A running Studio, and the handles a test needs to talk to it. */
export interface StudioApp {
  /** The CDP connection. Closing it detaches; it does not close the app. */
  browser: Browser;
  /** The throwaway copy of the document this launch opened, if any. */
  documentPath?: string;
  /** The Studio hub window. Fails if this launch never opened one. */
  hub(): Promise<Page>;
  /** The first Reader window. Fails if this launch never opened one. */
  reader(): Promise<Page>;
  /**
   * The sandboxed application's frame inside a Reader window. It is a
   * cross-origin `poid://` document and therefore an out-of-process iframe, so
   * it is reached as a frame of the reader page, never as a page of its own.
   */
  appFrame(reader: Page): Promise<import("@playwright/test").Frame>;
  /** Anything the application printed on stderr — the honest failure channel. */
  stderr(): string;
  /** Stops the application and removes its throwaway data directory. */
  close(): Promise<void>;
}

/** Resolves once the CDP endpoint answers, or throws with the app's own stderr. */
async function waitForCdp(child: ChildProcess, stderr: () => string): Promise<string> {
  const deadline = Date.now() + LAUNCH_TIMEOUT_MS;
  let lastError = "";
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `Studio exited with code ${child.exitCode} before opening a window.\n${stderr()}`,
      );
    }
    try {
      const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`);
      if (response.ok) return `http://127.0.0.1:${CDP_PORT}`;
      lastError = `HTTP ${response.status}`;
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e);
    }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  throw new Error(
    `The Studio webview never opened a debugging port on ${CDP_PORT} ` +
      `(last attempt: ${lastError}).\n${stderr()}`,
  );
}

/** Polls every context for a page whose URL matches, since windows open late. */
async function findPage(browser: Browser, match: RegExp, what: string): Promise<Page> {
  const deadline = Date.now() + LAUNCH_TIMEOUT_MS;
  while (Date.now() < deadline) {
    for (const context of browser.contexts()) {
      for (const page of context.pages()) {
        if (match.test(page.url())) {
          // The window exists; wait for its bundle to have run.
          await page.waitForLoadState("domcontentloaded");
          return page;
        }
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  const seen = browser
    .contexts()
    .flatMap((c) => c.pages().map((p) => p.url()))
    .join(", ");
  throw new Error(`No ${what} window appeared. Windows present: ${seen || "none"}`);
}

/**
 * Launches Studio and attaches to it.
 *
 * The environment does three things and nothing else: it opens the debugging
 * port, and it redirects both app-data variables so the vault, the connection
 * registry and the webview's own profile land in a directory this function
 * deletes afterwards.
 */
export async function launchStudio(options: LaunchOptions = {}): Promise<StudioApp> {
  assertNoRunningStudio();

  const exe = studioExecutable();
  const dataDir = mkdtempSync(join(tmpdir(), "poid-studio-e2e-"));

  // Open a *copy*, never the file the caller named.
  //
  // Opening a POID writes to it: the reader stamps `instance.id` into the
  // manifest on first open, which is how a copy of a document becomes a
  // document of its own (SPEC §3.2, §6.3). Correct behaviour — and it means
  // that pointing a test at a conformance fixture would rewrite a file the
  // repository generates deterministically and CI diffs. Copying also matches
  // reality: a user's documents do not live inside the source tree.
  let documentPath: string | undefined;
  if (options.open) {
    const documents = join(dataDir, "documents");
    mkdirSync(documents, { recursive: true });
    documentPath = join(documents, basename(options.open));
    copyFileSync(options.open, documentPath);
  }
  const args = documentPath ? [documentPath] : [];

  // Point the converter's build path at the locally staged engine and
  // Standard Library rather than the Cloudflare download: `fetch-esbuild.mjs`
  // stages the wasm from node_modules, and `@poid/stdlib` builds its bundles.
  // Absent files just leave the build-path test to skip itself.
  const stagedEsbuild = join(repoRoot, "engines", ".cache", "esbuild", "esbuild.wasm");
  const stdlibDir = join(repoRoot, "packages", "poid-stdlib", "lib");

  const child = spawn(exe, args, {
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
      APPDATA: join(dataDir, "Roaming"),
      LOCALAPPDATA: join(dataDir, "Local"),
      ...(existsSync(stagedEsbuild) ? { POID_ESBUILD_WASM: stagedEsbuild } : {}),
      ...(existsSync(stdlibDir) ? { POID_STDLIB: stdlibDir } : {}),
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: false,
  });

  let errors = "";
  child.stderr?.on("data", (chunk: Buffer) => {
    errors += chunk.toString();
  });
  child.stdout?.on("data", (chunk: Buffer) => {
    errors += chunk.toString();
  });
  const stderr = () => errors;

  let browser: Browser;
  try {
    const endpoint = await waitForCdp(child, stderr);
    browser = await chromium.connectOverCDP(endpoint);
  } catch (e) {
    child.kill();
    rmSync(dataDir, { recursive: true, force: true });
    throw e;
  }

  return {
    browser,
    documentPath,
    hub: () => findPage(browser, HUB_PAGE, "Studio hub"),
    reader: () => findPage(browser, READER_PAGE, "Reader"),
    appFrame: async (reader: Page) => {
      const deadline = Date.now() + LAUNCH_TIMEOUT_MS;
      while (Date.now() < deadline) {
        const frame = reader
          .frames()
          .find((f) => f !== reader.mainFrame() && /poid(?:\.localhost|:)/.test(f.url()));
        if (frame) return frame;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      throw new Error(
        `No sandboxed application frame appeared. Frames: ${reader
          .frames()
          .map((f) => f.url())
          .join(", ")}`,
      );
    },
    stderr,
    close: async () => {
      await browser.close().catch(() => undefined);
      if (child.pid !== undefined) {
        // The webview runs in child processes of its own; killing only the
        // parent leaves them holding the debugging port for the next test.
        try {
          execFileSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], { stdio: "ignore" });
        } catch {
          child.kill("SIGKILL");
        }
      }
      // Windows keeps the webview profile locked for a moment after exit.
      for (let attempt = 0; attempt < 10; attempt++) {
        try {
          rmSync(dataDir, { recursive: true, force: true });
          return;
        } catch {
          await new Promise((resolve) => setTimeout(resolve, 200));
        }
      }
    },
  };
}

/** A conformance fixture, by name — the containers the whole repo agrees on. */
export function fixture(name: string): string {
  return join(repoRoot, "spec", "conformance", name);
}
