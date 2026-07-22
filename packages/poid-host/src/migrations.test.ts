/**
 * Migrations: parsing the `migrations/` set, the schema-version state
 * machine (stamp fresh / migrate existing / refuse downgrade), atomicity of
 * a failed migration, and the end-to-end "update program, keep data" flow
 * driven through `makeSqlHandlers`.
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import "fake-indexeddb/auto";
import { beforeAll, describe, expect, it } from "vitest";
import { BrokerError, type ReaderSession } from "./broker.js";
import type { Scope } from "./engine.js";
import { applyMigrations, parseMigrations, schemaVersionOf } from "./migrations.js";
import { WaSqliteEngine } from "./sql-engine.js";
import { IdbSqlPersistence, makeSqlHandlers, type SqlHandlers } from "./sql-persistence.js";

const require = createRequire(import.meta.url);

let wasmBinary: ArrayBuffer;

beforeAll(async () => {
  const bytes = await readFile(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"));
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  wasmBinary = copy;
});

let seq = 0;
function freshScope(): Scope {
  return { instanceId: `mig-${seq++}`, slot: "" };
}

function file(sql: string): Uint8Array {
  return new TextEncoder().encode(sql);
}

describe("parseMigrations", () => {
  it("orders by numeric version and ignores non-migration files", () => {
    const files = new Map<string, Uint8Array>([
      ["migrations/0002-b.sql", file("SELECT 2")],
      ["migrations/0001-a.sql", file("SELECT 1")],
      ["migrations/0010-c.sql", file("SELECT 10")],
      ["app/index.html", file("<html>")],
      ["data/store.json", file("{}")],
    ]);
    const migrations = parseMigrations(files);
    expect(migrations.map((m) => m.version)).toEqual([1, 2, 10]);
    expect(migrations.map((m) => m.name)).toEqual(["a", "b", "c"]);
  });

  it("rejects a malformed name and duplicate numbers", () => {
    expect(() => parseMigrations(new Map([["migrations/nope.sql", file("")]]))).toThrow(
      BrokerError,
    );
    expect(() =>
      parseMigrations(
        new Map([
          ["migrations/0001-a.sql", file("")],
          ["migrations/0001-b.sql", file("")],
        ]),
      ),
    ).toThrow(BrokerError);
  });

  it("returns an empty list when there are no migrations", () => {
    expect(parseMigrations(new Map([["app/index.html", file("<html>")]]))).toEqual([]);
  });
});

describe("applyMigrations", () => {
  it("applies pending migrations in order and records the version", async () => {
    const engine = await WaSqliteEngine.create({ wasmBinary });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(id INTEGER PRIMARY KEY)");
    const migrations = [
      { version: 1, name: "add-name", sql: "ALTER TABLE t ADD COLUMN name TEXT" },
      { version: 2, name: "seed", sql: "INSERT INTO t(id, name) VALUES (1, 'x')" },
    ];
    const applied = await applyMigrations(engine, scope, 2, migrations);
    expect(applied).toEqual([1, 2]);
    expect(await schemaVersionOf(engine, scope)).toBe(2);
    const { rows } = await engine.exec(scope, "SELECT id, name FROM t");
    expect(rows).toEqual([{ id: 1, name: "x" }]);
  });

  it("only applies migrations above the current version and at or below target", async () => {
    const engine = await WaSqliteEngine.create({ wasmBinary });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(v); PRAGMA user_version = 2");
    const migrations = [
      { version: 1, name: "old", sql: "INSERT INTO t VALUES(1)" },
      { version: 2, name: "current", sql: "INSERT INTO t VALUES(2)" },
      { version: 3, name: "next", sql: "INSERT INTO t VALUES(3)" },
      { version: 4, name: "future", sql: "INSERT INTO t VALUES(4)" },
    ];
    const applied = await applyMigrations(engine, scope, 3, migrations);
    expect(applied).toEqual([3]);
    expect((await engine.exec(scope, "SELECT v FROM t ORDER BY v")).rows).toEqual([{ v: 3 }]);
  });

  it("rolls a failing migration back atomically, leaving the version untouched", async () => {
    const engine = await WaSqliteEngine.create({ wasmBinary });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(v)");
    const migrations = [
      { version: 1, name: "ok", sql: "INSERT INTO t VALUES(1)" },
      { version: 2, name: "boom", sql: "INSERT INTO t VALUES(2); INSERT INTO nope VALUES(3)" },
    ];
    await expect(applyMigrations(engine, scope, 2, migrations)).rejects.toMatchObject({
      code: "INTERNAL",
    });
    // Migration 1 committed; migration 2 rolled back entirely.
    expect(await schemaVersionOf(engine, scope)).toBe(1);
    expect((await engine.exec(scope, "SELECT v FROM t ORDER BY v")).rows).toEqual([{ v: 1 }]);
  });

  it("refuses a downgrade (data newer than the code)", async () => {
    const engine = await WaSqliteEngine.create({ wasmBinary });
    const scope = freshScope();
    await engine.exec(scope, "PRAGMA user_version = 5");
    await expect(applyMigrations(engine, scope, 3, [])).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });
  });
});

describe("update program, keep data (end to end)", () => {
  let seq2 = 0;
  function session(instanceId = `update-${seq2++}`): ReaderSession {
    return {
      instanceId,
      appInfo: {
        id: "com.example.kanban",
        name: "Kanban",
        version: "1.0.0",
        readerVersion: "0.0.1",
        platform: "web",
        storageMode: "embedded",
      },
      capabilities: new Set(["db.sql", "db.docs"]) as ReaderSession["capabilities"],
      slots: [],
      currentSlot: "",
      quotaBytes: 64 * 1024 * 1024,
    };
  }

  async function insertBoard(sql: SqlHandlers, s: ReaderSession): Promise<void> {
    await sql.sql(s, "db.sql.exec", {
      sql: "CREATE TABLE IF NOT EXISTS boards(id INTEGER, title TEXT)",
    });
    await sql.sql(s, "db.sql.exec", { sql: "INSERT INTO boards VALUES(1, 'Todo')" });
  }

  it("stamps a fresh v1 install, then migrates to v2 preserving data", async () => {
    const dbName = `update-e2e-${Date.now()}`;
    const persistence = await IdbSqlPersistence.open(dbName);
    const s = session("instance-x");

    // v1: schema_version 1, no migrations. Fresh install is stamped at 1.
    const v1 = makeSqlHandlers({ wasm: { wasmBinary }, persistence, schemaVersion: 1 });
    await insertBoard(v1, s);
    await v1.flush();

    // v2: schema_version 2, with a migration that adds a column. The same
    // instance's data database is still at version 1.
    const persistence2 = await IdbSqlPersistence.open(dbName);
    const v2 = makeSqlHandlers({
      wasm: { wasmBinary },
      persistence: persistence2,
      schemaVersion: 2,
      migrations: [
        {
          version: 2,
          name: "add-archived",
          sql: "ALTER TABLE boards ADD COLUMN archived INTEGER DEFAULT 0",
        },
      ],
    });
    const rows = (await v2.sql(s, "db.sql.exec", {
      sql: "SELECT id, title, archived FROM boards",
    })) as { rows: unknown[] };
    // The v1 board survived and picked up the new column's default.
    expect(rows.rows).toEqual([{ id: 1, title: "Todo", archived: 0 }]);
  });

  it("does not migrate a fresh v2 install (schema built by the app), only stamps it", async () => {
    const persistence = await IdbSqlPersistence.open(`update-fresh-${Date.now()}`);
    const s = session();
    const v2 = makeSqlHandlers({
      wasm: { wasmBinary },
      persistence,
      schemaVersion: 2,
      migrations: [
        // If this ran against an empty DB it would fail (no such table).
        { version: 2, name: "bad-on-empty", sql: "ALTER TABLE boards ADD COLUMN archived INTEGER" },
      ],
    });
    // A fresh install never runs the migration; the app builds its own schema.
    await insertBoard(v2, s);
    const count = (await v2.sql(s, "db.sql.exec", { sql: "SELECT COUNT(*) AS n FROM boards" })) as {
      rows: unknown[];
    };
    expect(count.rows).toEqual([{ n: 1 }]);
  });
});
