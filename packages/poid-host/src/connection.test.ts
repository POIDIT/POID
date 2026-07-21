/**
 * The connection seam: a loopback connection serves the same db.sql / db.docs
 * API as the local store, but keyed by connection id — so two different POID
 * instances configured against one connection share a database, and the same
 * app runs unchanged whichever mode it is in.
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import "fake-indexeddb/auto";
import { beforeAll, describe, expect, it } from "vitest";
import type { ReaderSession } from "./broker.js";
import { connectionScope, loopbackConnection } from "./connection.js";
import { IdbSqlPersistence } from "./sql-persistence.js";

const require = createRequire(import.meta.url);

let wasmBinary: ArrayBuffer;

beforeAll(async () => {
  const bytes = await readFile(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"));
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  wasmBinary = copy;
});

let seq = 0;
/** A session for a distinct POID instance (distinct window). */
function session(instanceId = `inst-${seq++}`): ReaderSession {
  return {
    instanceId,
    appInfo: {
      id: "com.example.kanban",
      name: "Kanban",
      version: "1.0.0",
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: "connection",
    },
    capabilities: new Set(["db.sql", "db.docs"]) as ReaderSession["capabilities"],
    slots: [],
    currentSlot: "",
    quotaBytes: 64 * 1024 * 1024,
    connections: new Map(),
  };
}

describe("loopbackConnection", () => {
  it("shares one database across two different instances of the connection", async () => {
    const wasm = { wasmBinary };
    const conn = loopbackConnection({ connectionId: "conn-A", wasm });

    // Instance 1 inserts a board.
    const s1 = session("instance-1");
    await conn.docs(s1, "db.docs.insert", { name: "boards", doc: { _id: "b1", title: "Todo" } });

    // Instance 2 — a completely separate window/file — sees it, because the
    // data lives with the connection, not the instance.
    const s2 = session("instance-2");
    const found = (await conn.docs(s2, "db.docs.find", { name: "boards", query: {} })) as unknown[];
    expect(found).toEqual([{ _id: "b1", title: "Todo" }]);
  });

  it("keeps two different connections apart", async () => {
    const wasm = { wasmBinary };
    const a = loopbackConnection({ connectionId: "conn-X", wasm });
    const b = loopbackConnection({ connectionId: "conn-Y", wasm });
    await a.sql(session(), "db.sql.exec", { sql: "CREATE TABLE t(v); INSERT INTO t VALUES(1)" });
    // Connection Y has never seen that table.
    await expect(b.sql(session(), "db.sql.exec", { sql: "SELECT * FROM t" })).rejects.toMatchObject(
      {
        code: "INVALID_ARGUMENT",
      },
    );
  });

  it("persists per connection id and survives a restart", async () => {
    const wasm = { wasmBinary };
    const dbName = `conn-persist-${Date.now()}`;
    const persistence = await IdbSqlPersistence.open(dbName);

    const first = loopbackConnection({ connectionId: "shared", wasm, persistence });
    await first.docs(session(), "db.docs.insert", { name: "c", doc: { _id: "1", v: 42 } });
    await first.flush();

    // A restart: fresh handlers, same connection id and persistence store.
    const persistence2 = await IdbSqlPersistence.open(dbName);
    const restarted = loopbackConnection({
      connectionId: "shared",
      wasm,
      persistence: persistence2,
    });
    const found = (await restarted.docs(session(), "db.docs.find", {
      name: "c",
      query: { _id: "1" },
    })) as unknown[];
    expect(found).toEqual([{ _id: "1", v: 42 }]);
  });

  it("does not store under any instance id (data never travels with a file)", async () => {
    const wasm = { wasmBinary };
    const persistence = await IdbSqlPersistence.open(`conn-scope-${Date.now()}`);
    const conn = loopbackConnection({ connectionId: "scoped", wasm, persistence });
    const s = session("instance-42");
    await conn.sql(s, "db.sql.exec", { sql: "CREATE TABLE t(v)" });
    await conn.flush();

    // The bytes live under the connection scope, and nothing under the
    // instance id the app was opened from.
    expect(await persistence.load(connectionScope("scoped"))).toBeDefined();
    expect(await persistence.load({ instanceId: "instance-42", slot: "" })).toBeUndefined();
  });
});
