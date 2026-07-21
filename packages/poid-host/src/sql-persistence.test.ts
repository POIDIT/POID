/**
 * Wiring tier: lazy engine wake-up, per-scope seeding from persistence,
 * flush/snapshot lifecycle, and the IndexedDB persistence port (via
 * fake-indexeddb, like the kv engine's tests).
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import "fake-indexeddb/auto";
import { beforeAll, describe, expect, it } from "vitest";
import type { ReaderSession } from "./broker.js";
import type { Scope } from "./engine.js";
import { IdbSqlPersistence, makeSqlHandlers, type SqlPersistence } from "./sql-persistence.js";

const require = createRequire(import.meta.url);

let wasmBinary: ArrayBuffer;

beforeAll(async () => {
  const bytes = await readFile(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"));
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  wasmBinary = copy;
});

let seq = 0;
function makeSession(capabilities: string[] = ["db.sql", "db.docs"]): ReaderSession {
  return {
    instanceId: `wired-${seq++}`,
    appInfo: {
      id: "com.example.test",
      name: "Test",
      version: "1.0.0",
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: "vault",
    },
    capabilities: new Set(capabilities) as ReaderSession["capabilities"],
    slots: [],
    currentSlot: "",
    quotaBytes: 64 * 1024 * 1024,
    connections: new Map(),
  };
}

class MemoryPersistence implements SqlPersistence {
  readonly map = new Map<string, Uint8Array>();
  loads = 0;
  async load(scope: Scope): Promise<Uint8Array | undefined> {
    this.loads += 1;
    return this.map.get(`${scope.instanceId}|${scope.slot}`);
  }
  async persist(scope: Scope, bytes: Uint8Array): Promise<void> {
    this.map.set(`${scope.instanceId}|${scope.slot}`, bytes);
  }
}

describe("makeSqlHandlers", () => {
  it("wires sql and docs through one lazy engine and persists committed state", async () => {
    const persistence = new MemoryPersistence();
    const handlers = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const session = makeSession();

    await handlers.sql(session, "db.sql.exec", { sql: "CREATE TABLE t(x)" });
    await handlers.sql(session, "db.sql.exec", { sql: "INSERT INTO t VALUES(1)" });
    await handlers.docs(session, "db.docs.insert", { name: "c", doc: { title: "x" } });
    await handlers.flush();

    // A fresh set of handlers — a reader restart — sees both tiers' data.
    const restarted = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const rows = (await restarted.sql(session, "db.sql.exec", {
      sql: "SELECT COUNT(*) AS n FROM t",
    })) as { rows: unknown[] };
    expect(rows.rows).toEqual([{ n: 1 }]);
    expect(await restarted.docs(session, "db.docs.count", { name: "c" })).toBe(1);
  });

  it("seeds each scope exactly once", async () => {
    const persistence = new MemoryPersistence();
    const handlers = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const session = makeSession();
    await Promise.all([
      handlers.sql(session, "db.sql.exec", { sql: "SELECT 1" }),
      handlers.sql(session, "db.sql.exec", { sql: "SELECT 2" }),
      handlers.docs(session, "db.docs.count", { name: "c" }),
    ]);
    expect(persistence.loads).toBe(1);
  });

  it("flush is a no-op when the engine never woke", async () => {
    const handlers = makeSqlHandlers({ wasm: { wasmBinary } });
    await expect(handlers.flush()).resolves.toBeUndefined();
  });

  it("snapshot falls back to persisted bytes without waking the engine", async () => {
    const persistence = new MemoryPersistence();
    const seedHandlers = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const session = makeSession();
    await seedHandlers.sql(session, "db.sql.exec", { sql: "CREATE TABLE t(x)" });
    await seedHandlers.flush();

    const idle = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const bytes = await idle.snapshot({ instanceId: session.instanceId, slot: "" });
    expect(bytes).not.toBeNull();
    expect(persistence.loads).toBeGreaterThan(0);

    const empty = await idle.snapshot({ instanceId: "never-used", slot: "" });
    expect(empty).toBeNull();
  });

  it("dump() returns null without waking the engine when there is no SQL data", async () => {
    // The kv-only download path calls dump(); it must not load the ~550 KB
    // engine (which, in the browser, would fetch wa-sqlite.wasm) for nothing.
    const persistence = new MemoryPersistence();
    const handlers = makeSqlHandlers({ wasm: { wasmBinary }, persistence });
    const dump = await handlers.dump({ instanceId: "kv-only", slot: "" });
    expect(dump).toBeNull();
    // Only the cheap persistence probe ran — the engine never woke.
    expect(persistence.loads).toBe(1);
  });
});

describe("IdbSqlPersistence", () => {
  it("round-trips bytes per (instanceId, slot) and removes them", async () => {
    const idb = await IdbSqlPersistence.open(`sql-test-${Date.now()}`);
    const a: Scope = { instanceId: "i1", slot: "" };
    const b: Scope = { instanceId: "i1", slot: "other" };
    await idb.persist(a, new Uint8Array([1, 2, 3]));
    await idb.persist(b, new Uint8Array([9]));
    expect(await idb.load(a)).toEqual(new Uint8Array([1, 2, 3]));
    expect(await idb.load(b)).toEqual(new Uint8Array([9]));
    await idb.remove(a);
    expect(await idb.load(a)).toBeUndefined();
    expect(await idb.load(b)).toEqual(new Uint8Array([9]));
  });
});
