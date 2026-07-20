/**
 * WaSqliteEngine unit tier: correctness of execution, the transaction gate,
 * scope isolation (including the ATTACH probe), quota, persistence, and the
 * broker handler adapter. Runs the real wa-sqlite WASM build in Node.
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { beforeAll, describe, expect, it } from "vitest";
import { BrokerError, type ReaderSession } from "./broker.js";
import type { Scope } from "./engine.js";
import { type SqlEngineOptions, sqlBrokerHandler, WaSqliteEngine } from "./sql-engine.js";

const require = createRequire(import.meta.url);

let wasmBinary: ArrayBuffer;

beforeAll(async () => {
  const path = require.resolve("wa-sqlite/dist/wa-sqlite.wasm");
  const bytes = await readFile(path);
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  wasmBinary = copy;
});

function makeEngine(options: SqlEngineOptions = {}): Promise<WaSqliteEngine> {
  return WaSqliteEngine.create({ wasmBinary }, options);
}

let scopeSeq = 0;
function freshScope(slot = ""): Scope {
  return { instanceId: `instance-${scopeSeq++}`, slot };
}

async function expectBrokerError(promise: Promise<unknown>, code: string): Promise<BrokerError> {
  try {
    await promise;
  } catch (err) {
    expect(err).toBeInstanceOf(BrokerError);
    const broker = err as BrokerError;
    expect(broker.code).toBe(code);
    return broker;
  }
  throw new Error(`expected a BrokerError with code ${code}`);
}

describe("execution", () => {
  it("runs DDL, DML and SELECT with typed values", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(a, b, c, d, e)");
    const insert = await engine.exec(scope, "INSERT INTO t VALUES(?, ?, ?, ?, ?)", [
      42,
      1.5,
      "text",
      null,
      new Uint8Array([1, 2, 3]),
    ]);
    expect(insert.rowsAffected).toBe(1);
    const { rows } = await engine.exec(scope, "SELECT * FROM t");
    expect(rows).toHaveLength(1);
    const row = rows[0] as Record<string, unknown>;
    expect(row.a).toBe(42);
    expect(row.b).toBe(1.5);
    expect(row.c).toBe("text");
    expect(row.d).toBeNull();
    expect(row.e).toEqual(new Uint8Array([1, 2, 3]));
  });

  it("reports rowsAffected for UPDATE and DELETE, 0 for SELECT and DDL", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    const ddl = await engine.exec(scope, "CREATE TABLE t(x)");
    expect(ddl.rowsAffected).toBe(0);
    await engine.exec(scope, "INSERT INTO t VALUES(1),(2),(3)");
    const update = await engine.exec(scope, "UPDATE t SET x = x + 1 WHERE x >= 2");
    expect(update.rowsAffected).toBe(2);
    const select = await engine.exec(scope, "SELECT * FROM t");
    expect(select.rowsAffected).toBe(0);
    const del = await engine.exec(scope, "DELETE FROM t");
    expect(del.rowsAffected).toBe(3);
  });

  it("runs multi-statement SQL without params; rows come from the last SELECT", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    const result = await engine.exec(
      scope,
      "CREATE TABLE t(x); INSERT INTO t VALUES(7); SELECT x FROM t",
    );
    expect(result.rows).toEqual([{ x: 7 }]);
    expect(result.rowsAffected).toBe(1);
  });

  it("rejects params with multi-statement SQL without executing anything", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    await expectBrokerError(
      engine.exec(scope, "INSERT INTO t VALUES(?); DROP TABLE t", [1]),
      "INVALID_ARGUMENT",
    );
    // Neither statement ran: the table exists and is empty.
    const { rows } = await engine.exec(scope, "SELECT COUNT(*) AS n FROM t");
    expect(rows).toEqual([{ n: 0 }]);
  });

  it("rejects a params-count mismatch", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x, y)");
    await expectBrokerError(
      engine.exec(scope, "INSERT INTO t VALUES(?, ?)", [1]),
      "INVALID_ARGUMENT",
    );
  });

  it("rejects non-bindable params", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    await expectBrokerError(
      engine.exec(scope, "INSERT INTO t VALUES(?)", [{ nested: true }]),
      "INVALID_ARGUMENT",
    );
  });

  it("binds safe integers exactly and booleans as 0/1", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(a, b, c)");
    await engine.exec(scope, "INSERT INTO t VALUES(?, ?, ?)", [
      Number.MAX_SAFE_INTEGER,
      true,
      false,
    ]);
    const { rows } = await engine.exec(scope, "SELECT a, b, c, typeof(a) AS ta FROM t");
    expect(rows[0]).toEqual({ a: Number.MAX_SAFE_INTEGER, b: 1, c: 0, ta: "integer" });
  });

  it("refuses to return a 64-bit integer outside the safe range", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    const err = await expectBrokerError(
      engine.exec(scope, "SELECT 9223372036854775807 AS big"),
      "INVALID_ARGUMENT",
    );
    expect(err.message).toContain("CAST");
  });

  it("surfaces SQL errors as INVALID_ARGUMENT", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await expectBrokerError(engine.exec(scope, "SELECT * FROM missing"), "INVALID_ARGUMENT");
    await expectBrokerError(engine.exec(scope, "NOT VALID SQL"), "INVALID_ARGUMENT");
  });

  it("caps result rows", async () => {
    const engine = await makeEngine({ maxResultRows: 10 });
    const scope = freshScope();
    await engine.exec(
      scope,
      "CREATE TABLE t(x); " +
        "WITH RECURSIVE s(v) AS (SELECT 1 UNION ALL SELECT v + 1 FROM s WHERE v < 100) " +
        "INSERT INTO t SELECT v FROM s",
    );
    await expectBrokerError(engine.exec(scope, "SELECT * FROM t"), "QUOTA_EXCEEDED");
    const paged = await engine.exec(scope, "SELECT * FROM t LIMIT 10");
    expect(paged.rows).toHaveLength(10);
  });
});

describe("transactions", () => {
  it("commits and rolls back through the API", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");

    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });
    await engine.commit(scope, tx);
    expect((await engine.exec(scope, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 1 }]);

    const tx2 = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(2)", undefined, { txId: tx2 });
    await engine.rollback(scope, tx2);
    expect((await engine.exec(scope, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 1 }]);
  });

  it("rejects a foreign or stale transaction token", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    const tx = await engine.begin(scope);
    await expectBrokerError(
      engine.exec(scope, "SELECT 1", undefined, { txId: "not-a-real-token" }),
      "INVALID_ARGUMENT",
    );
    await engine.commit(scope, tx);
    await expectBrokerError(engine.commit(scope, tx), "INVALID_ARGUMENT");
  });

  it("makes a non-tx exec wait for the open transaction instead of interleaving", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");

    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });

    // Outside the transaction: must not see the uncommitted row, and must
    // not resolve until the transaction ends.
    let outsideDone = false;
    const outside = engine.exec(scope, "SELECT COUNT(*) AS n FROM t").then((r) => {
      outsideDone = true;
      return r;
    });
    await new Promise((r) => setTimeout(r, 25));
    expect(outsideDone).toBe(false);

    await engine.commit(scope, tx);
    expect((await outside).rows).toEqual([{ n: 1 }]);
  });

  it("rolls back an idle transaction after the timeout", async () => {
    const engine = await makeEngine({ txIdleTimeoutMs: 30 });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });
    await new Promise((r) => setTimeout(r, 120));
    await expectBrokerError(engine.commit(scope, tx), "INVALID_ARGUMENT");
    expect((await engine.exec(scope, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 0 }]);
  });

  it("rejects manual BEGIN in exec and leaves no dangling transaction", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await expectBrokerError(engine.exec(scope, "BEGIN"), "INVALID_ARGUMENT");
    // The connection is back in autocommit: a plain exec works.
    const { rows } = await engine.exec(scope, "SELECT 1 AS one");
    expect(rows).toEqual([{ one: 1 }]);
  });

  it("allows a self-contained BEGIN..COMMIT batch", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    const result = await engine.exec(
      scope,
      "CREATE TABLE t(x); BEGIN; INSERT INTO t VALUES(1); COMMIT; SELECT COUNT(*) AS n FROM t",
    );
    expect(result.rows).toEqual([{ n: 1 }]);
  });

  it("detects a manual COMMIT smuggled into an API transaction", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    const tx = await engine.begin(scope);
    await expectBrokerError(
      engine.exec(scope, "INSERT INTO t VALUES(1); COMMIT", undefined, { txId: tx }),
      "INVALID_ARGUMENT",
    );
    // The engine transaction is gone; the token is dead.
    await expectBrokerError(engine.commit(scope, tx), "INVALID_ARGUMENT");
  });

  it("closes the transaction when a statement error makes SQLite roll back", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x PRIMARY KEY) WITHOUT ROWID");
    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });
    // ON CONFLICT ROLLBACK ends the whole transaction inside SQLite.
    await expectBrokerError(
      engine.exec(scope, "INSERT OR ROLLBACK INTO t VALUES(1)", undefined, { txId: tx }),
      "INVALID_ARGUMENT",
    );
    await expectBrokerError(engine.commit(scope, tx), "INVALID_ARGUMENT");
    // Nothing from the transaction survived.
    expect((await engine.exec(scope, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 0 }]);
  });
});

describe("scope isolation", () => {
  it("keeps two scopes invisible to each other", async () => {
    const engine = await makeEngine();
    const a = freshScope();
    const b = freshScope();
    await engine.exec(a, "CREATE TABLE secrets(v); INSERT INTO secrets VALUES('a-data')");
    await expectBrokerError(engine.exec(b, "SELECT * FROM secrets"), "INVALID_ARGUMENT");
  });

  it("keeps two slots of one instance apart", async () => {
    const engine = await makeEngine();
    const instanceId = "shared-instance";
    await engine.exec({ instanceId, slot: "one" }, "CREATE TABLE t(x)");
    await expectBrokerError(
      engine.exec({ instanceId, slot: "two" }, "SELECT * FROM t"),
      "INVALID_ARGUMENT",
    );
  });

  it("ATTACH cannot reach another scope's database", async () => {
    const engine = await makeEngine();
    const victim = freshScope();
    const attacker = freshScope();
    await engine.exec(victim, "CREATE TABLE loot(v); INSERT INTO loot VALUES('gold')");

    // The attacker attaches the exact file name the victim's database uses
    // inside its own VFS — and gets a fresh empty database, not the victim's.
    await engine.exec(attacker, "ATTACH DATABASE 'main.db' AS stolen");
    await expectBrokerError(engine.exec(attacker, "SELECT * FROM stolen.loot"), "INVALID_ARGUMENT");

    // The victim's data is untouched.
    const { rows } = await engine.exec(victim, "SELECT v FROM loot");
    expect(rows).toEqual([{ v: "gold" }]);
  });
});

describe("quota", () => {
  it("maps SQLITE_FULL to QUOTA_EXCEEDED", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    // ~64 KB quota: 16 pages of 4 KB.
    await engine.exec(scope, "CREATE TABLE t(x)", undefined, { quotaBytes: 64 * 1024 });
    await expectBrokerError(
      engine.exec(
        scope,
        "WITH RECURSIVE s(v) AS (SELECT 1 UNION ALL SELECT v + 1 FROM s WHERE v < 200) " +
          "INSERT INTO t SELECT zeroblob(4000) FROM s",
      ),
      "QUOTA_EXCEEDED",
    );
  });
});

describe("persistence", () => {
  it("persists committed bytes write-behind and round-trips through load()", async () => {
    const persisted = new Map<string, Uint8Array>();
    const engine = await makeEngine({
      persist: (scope, bytes) => {
        persisted.set(scope.instanceId, bytes);
      },
    });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x); INSERT INTO t VALUES(41), (1)");
    await engine.flush();
    const bytes = persisted.get(scope.instanceId);
    expect(bytes).toBeDefined();
    expect(bytes?.byteLength).toBeGreaterThan(0);

    // A second engine — a restart — loads the snapshot and sees the data.
    const engine2 = await makeEngine();
    const reopened = freshScope();
    engine2.load(reopened, bytes as Uint8Array);
    const { rows } = await engine2.exec(reopened, "SELECT SUM(x) AS total FROM t");
    expect(rows).toEqual([{ total: 42 }]);
  });

  it("does not persist uncommitted transaction state; commit persists it", async () => {
    const snapshots: Uint8Array[] = [];
    const engine = await makeEngine({
      persist: (_scope, bytes) => {
        snapshots.push(bytes);
      },
    });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    await engine.flush();
    const beforeTx = snapshots.length;

    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });
    await engine.flush();
    expect(snapshots.length).toBe(beforeTx);

    await engine.commit(scope, tx);
    await engine.flush();
    expect(snapshots.length).toBeGreaterThan(beforeTx);

    const engine2 = await makeEngine();
    const reopened = freshScope();
    engine2.load(reopened, snapshots[snapshots.length - 1] as Uint8Array);
    expect((await engine2.exec(reopened, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 1 }]);
  });

  it("SELECTs do not schedule persists", async () => {
    let calls = 0;
    const engine = await makeEngine({
      persist: () => {
        calls += 1;
      },
    });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    await engine.flush();
    const after = calls;
    await engine.exec(scope, "SELECT * FROM t");
    await engine.flush();
    expect(calls).toBe(after);
  });

  it("flush rethrows a persist failure once", async () => {
    const engine = await makeEngine({
      persist: () => {
        throw new Error("disk on fire");
      },
    });
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    await expect(engine.flush()).rejects.toThrow("disk on fire");
    await expect(engine.flush()).resolves.toBeUndefined();
  });

  it("snapshot() waits for an open transaction", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "CREATE TABLE t(x)");
    const tx = await engine.begin(scope);
    await engine.exec(scope, "INSERT INTO t VALUES(1)", undefined, { txId: tx });

    let snapshotDone = false;
    const snapshot = engine.snapshot(scope).then((b) => {
      snapshotDone = true;
      return b;
    });
    await new Promise((r) => setTimeout(r, 25));
    expect(snapshotDone).toBe(false);
    await engine.rollback(scope, tx);

    const bytes = await snapshot;
    const engine2 = await makeEngine();
    const reopened = freshScope();
    engine2.load(reopened, bytes);
    expect((await engine2.exec(reopened, "SELECT COUNT(*) AS n FROM t")).rows).toEqual([{ n: 0 }]);
  });

  it("load() on an open scope is a host bug and says so", async () => {
    const engine = await makeEngine();
    const scope = freshScope();
    await engine.exec(scope, "SELECT 1");
    expect(() => engine.load(scope, new Uint8Array())).toThrow(BrokerError);
  });
});

describe("scalar functions", () => {
  it("registers reader-side functions on every scope connection", async () => {
    const engine = await makeEngine({
      functions: [
        {
          name: "regexp",
          nArg: 2,
          fn: ([pattern, value]) =>
            typeof pattern === "string" && typeof value === "string"
              ? new RegExp(pattern).test(value)
              : null,
        },
      ],
    });
    const scope = freshScope();
    const { rows } = await engine.exec(scope, "SELECT 'kanban-42' REGEXP '^kanban-\\d+$' AS m");
    expect(rows).toEqual([{ m: 1 }]);
  });

  it("degrades a throwing function to NULL instead of crossing the WASM stack", async () => {
    const engine = await makeEngine({
      functions: [
        {
          name: "boom",
          nArg: 0,
          fn: () => {
            throw new Error("nope");
          },
        },
      ],
    });
    const scope = freshScope();
    const { rows } = await engine.exec(scope, "SELECT boom() AS b");
    expect(rows).toEqual([{ b: null }]);
  });
});

describe("broker handler", () => {
  function makeSession(instanceId: string, capabilities: string[]): ReaderSession {
    return {
      instanceId,
      appInfo: {
        id: "com.example.test",
        name: "Test",
        version: "1.0.0",
        readerVersion: "0.0.1",
        platform: "web",
        storageMode: "embedded",
      },
      capabilities: new Set(capabilities) as ReaderSession["capabilities"],
      slots: [],
      currentSlot: "",
      quotaBytes: 64 * 1024 * 1024,
      connections: new Map(),
    };
  }

  it("dispatches exec/begin/commit and derives scope from the session", async () => {
    const engine = await makeEngine();
    const handler = sqlBrokerHandler(engine);
    const session = makeSession("handler-instance", ["db.sql"]);

    await handler(session, "db.sql.exec", { sql: "CREATE TABLE t(x)" });
    const txId = (await handler(session, "db.sql.begin", {})) as string;
    await handler(session, "db.sql.exec", { sql: "INSERT INTO t VALUES(1)", txId });
    await handler(session, "db.sql.commit", { txId });
    const result = (await handler(session, "db.sql.exec", {
      sql: "SELECT COUNT(*) AS n FROM t",
    })) as { rows: unknown[] };
    expect(result.rows).toEqual([{ n: 1 }]);

    // The same engine, addressed via the session's instance id.
    expect(
      (
        await engine.exec(
          { instanceId: "handler-instance", slot: "" },
          "SELECT COUNT(*) AS n FROM t",
        )
      ).rows,
    ).toEqual([{ n: 1 }]);
  });

  it("refuses without the db.sql capability", async () => {
    const engine = await makeEngine();
    const handler = sqlBrokerHandler(engine);
    const session = makeSession("nocap-instance", ["db.kv"]);
    await expectBrokerError(
      handler(session, "db.sql.exec", { sql: "SELECT 1" }),
      "PERMISSION_DENIED",
    );
  });

  it("validates argument shapes", async () => {
    const engine = await makeEngine();
    const handler = sqlBrokerHandler(engine);
    const session = makeSession("shape-instance", ["db.sql"]);
    await expectBrokerError(handler(session, "db.sql.exec", { sql: 42 }), "INVALID_ARGUMENT");
    await expectBrokerError(
      handler(session, "db.sql.exec", { sql: "SELECT 1", params: "not-an-array" }),
      "INVALID_ARGUMENT",
    );
    await expectBrokerError(handler(session, "db.sql.commit", {}), "INVALID_ARGUMENT");
  });
});
