/**
 * SQL text dump: the canonical embedded form must round-trip a database
 * through `data/database.sql` — every schema object and every row, with
 * exact type fidelity (floats, blobs, 64-bit integers, NULLs).
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { beforeAll, describe, expect, it } from "vitest";
import type { Scope } from "./engine.js";
import { dumpSql } from "./sql-dump.js";
import { WaSqliteEngine } from "./sql-engine.js";

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
  return { instanceId: `dump-${seq++}`, slot: "" };
}

function makeEngine(): Promise<WaSqliteEngine> {
  return WaSqliteEngine.create({ wasmBinary });
}

describe("dumpSql", () => {
  it("returns null for an empty database", async () => {
    const engine = await makeEngine();
    expect(await dumpSql(engine, freshScope())).toBeNull();
  });

  it("round-trips schema and rows with exact type fidelity", async () => {
    const engine = await makeEngine();
    const source = freshScope();
    await engine.exec(
      source,
      "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT, price REAL, blob BLOB, note TEXT); " +
        "CREATE INDEX idx_name ON items(name)",
    );
    await engine.exec(source, "INSERT INTO items VALUES(?, ?, ?, ?, ?)", [
      1,
      "wid'get",
      2.5,
      new Uint8Array([0, 1, 255]),
      null,
    ]);
    await engine.exec(source, "INSERT INTO items VALUES(?, ?, ?, ?, ?)", [
      9223372036854775807n,
      "big",
      -0.125,
      new Uint8Array([]),
      "ok",
    ]);

    const dump = await dumpSql(engine, source);
    expect(dump).toContain("CREATE TABLE items");
    expect(dump).toContain("CREATE INDEX idx_name");

    // Replay into a fresh scope and compare column-for-column.
    const target = freshScope();
    const engine2 = await makeEngine();
    await engine2.exec(target, dump as string);

    // Read the id as text: the 64-bit value exceeds JS's safe range and the
    // engine (correctly) refuses to hand it back as a number.
    const rows = await engine2.exec(
      target,
      "SELECT CAST(id AS TEXT) AS id, name, price, note, typeof(id) AS tid, hex(blob) AS bhex " +
        "FROM items ORDER BY id",
    );
    expect(rows.rows).toEqual([
      { id: "1", name: "wid'get", price: 2.5, note: null, tid: "integer", bhex: "0001FF" },
      {
        id: "9223372036854775807",
        name: "big",
        price: -0.125,
        note: "ok",
        tid: "integer",
        bhex: "",
      },
    ]);
  });

  it("preserves AUTOINCREMENT sequence counters", async () => {
    const engine = await makeEngine();
    const source = freshScope();
    await engine.exec(source, "CREATE TABLE t(id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT)");
    await engine.exec(source, "INSERT INTO t(v) VALUES('a'),('b'),('c')");
    await engine.exec(source, "DELETE FROM t WHERE id >= 2");
    const dump = await dumpSql(engine, source);

    const target = freshScope();
    const engine2 = await makeEngine();
    await engine2.exec(target, dump as string);
    // The next insert continues past the high-water mark, not from 2.
    await engine2.exec(target, "INSERT INTO t(v) VALUES('d')");
    const rows = await engine2.exec(target, "SELECT id FROM t WHERE v='d'");
    expect(rows.rows).toEqual([{ id: 4 }]);
  });

  it("dumps a large table across paging boundaries", async () => {
    const engine = await makeEngine();
    const source = freshScope();
    await engine.exec(
      source,
      "CREATE TABLE big(x); " +
        "WITH RECURSIVE s(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM s WHERE v < 9000) " +
        "INSERT INTO big SELECT v FROM s",
    );
    const dump = await dumpSql(engine, source);
    const target = freshScope();
    const engine2 = await makeEngine();
    await engine2.exec(target, dump as string);
    const rows = await engine2.exec(target, "SELECT COUNT(*) AS n, SUM(x) AS total FROM big");
    expect(rows.rows).toEqual([{ n: 9000, total: (9000 * 9001) / 2 }]);
  });
});
