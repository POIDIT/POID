/**
 * DocsStore unit tier: CRUD semantics, the shallow-merge update contract,
 * collection/scope isolation, name validation, and persistence round-trips.
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { beforeAll, describe, expect, it } from "vitest";
import { BrokerError, type ReaderSession } from "./broker.js";
import { DocsStore, docsBrokerHandler, docsRegexpFunction } from "./docs-store.js";
import type { Scope } from "./engine.js";
import { type SqlEngineOptions, WaSqliteEngine } from "./sql-engine.js";

const require = createRequire(import.meta.url);

let wasmBinary: ArrayBuffer;

beforeAll(async () => {
  const bytes = await readFile(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"));
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  wasmBinary = copy;
});

async function makeStore(
  options: SqlEngineOptions = {},
): Promise<{ engine: WaSqliteEngine; store: DocsStore }> {
  const engine = await WaSqliteEngine.create(
    { wasmBinary },
    { functions: [docsRegexpFunction], ...options },
  );
  return { engine, store: new DocsStore(engine) };
}

let scopeSeq = 0;
function freshScope(): Scope {
  return { instanceId: `docs-instance-${scopeSeq++}`, slot: "" };
}

async function expectCode(promise: Promise<unknown>, code: string): Promise<void> {
  try {
    await promise;
  } catch (err) {
    expect(err).toBeInstanceOf(BrokerError);
    expect((err as BrokerError).code).toBe(code);
    return;
  }
  throw new Error(`expected a BrokerError with code ${code}`);
}

describe("insert and find", () => {
  it("inserts, generates a UUID _id, and finds by field", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    const { _id } = await store.insert(scope, "boards", { title: "Todo", order: 1 });
    expect(_id).toMatch(/^[0-9a-f-]{36}$/);
    const docs = await store.find(scope, "boards", { title: "Todo" });
    expect(docs).toEqual([{ _id, title: "Todo", order: 1 }]);
  });

  it("honours a caller-supplied string _id and rejects duplicates", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "mine", v: 1 });
    expect(await store.findOne(scope, "c", { _id: "mine" })).toEqual({ _id: "mine", v: 1 });
    await expectCode(store.insert(scope, "c", { _id: "mine", v: 2 }), "INVALID_ARGUMENT");
  });

  it("rejects non-object documents and non-string _id", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await expectCode(store.insert(scope, "c", [1, 2]), "INVALID_ARGUMENT");
    await expectCode(store.insert(scope, "c", "text"), "INVALID_ARGUMENT");
    await expectCode(store.insert(scope, "c", { _id: 42 }), "INVALID_ARGUMENT");
  });

  it("sorts, skips and limits deterministically", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    for (const [i, title] of ["b", "a", "c", "a"].entries()) {
      await store.insert(scope, "c", { _id: `d${i}`, title, i });
    }
    const sorted = await store.find(scope, "c", {}, { sort: { title: 1 } });
    expect(sorted.map((d) => d._id)).toEqual(["d1", "d3", "d0", "d2"]);
    const page = await store.find(scope, "c", {}, { sort: { title: 1 }, skip: 1, limit: 2 });
    expect(page.map((d) => d._id)).toEqual(["d3", "d0"]);
    const desc = await store.find(scope, "c", {}, { sort: { i: -1 }, limit: 1 });
    expect(desc.map((d) => d._id)).toEqual(["d3"]);
  });

  it("findOne returns null when nothing matches", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    expect(await store.findOne(scope, "c", { missing: true })).toBeNull();
  });

  it("supports $regex through the registered regexp function", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", tag: "kanban-42" });
    await store.insert(scope, "c", { _id: "2", tag: "note" });
    const hits = await store.find(scope, "c", { tag: { $regex: "^kanban-\\d+$" } });
    expect(hits.map((d) => d._id)).toEqual(["1"]);
  });

  it("queries nested fields and array indices with dot paths", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", nested: { x: 5 }, tags: ["red", "blue"] });
    await store.insert(scope, "c", { _id: "2", nested: { x: 6 }, tags: ["blue"] });
    expect((await store.find(scope, "c", { "nested.x": 5 })).map((d) => d._id)).toEqual(["1"]);
    expect((await store.find(scope, "c", { "tags.0": "blue" })).map((d) => d._id)).toEqual(["2"]);
  });
});

describe("update", () => {
  it("shallow-merges the patch and leaves other fields intact", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", title: "Todo", order: 1, done: false });
    const { matched } = await store.update(scope, "c", { _id: "1" }, { done: true, order: 2 });
    expect(matched).toBe(1);
    expect(await store.findOne(scope, "c", { _id: "1" })).toEqual({
      _id: "1",
      title: "Todo",
      order: 2,
      done: true,
    });
  });

  it("updates nested fields via dotted patch keys", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", meta: { color: "red", pinned: true } });
    await store.update(scope, "c", { _id: "1" }, { "meta.color": "blue" });
    expect(await store.findOne(scope, "c", { _id: "1" })).toEqual({
      _id: "1",
      meta: { color: "blue", pinned: true },
    });
  });

  it("counts matches without changes for an empty patch", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", v: 1 });
    await store.insert(scope, "c", { _id: "2", v: 2 });
    expect(await store.update(scope, "c", { v: { $gte: 1 } }, {})).toEqual({ matched: 2 });
    expect(await store.findOne(scope, "c", { _id: "1" })).toEqual({ _id: "1", v: 1 });
  });

  it("rejects update operators and _id changes", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await expectCode(store.update(scope, "c", {}, { $set: { a: 1 } }), "INVALID_ARGUMENT");
    await expectCode(store.update(scope, "c", {}, { _id: "other" }), "INVALID_ARGUMENT");
  });

  it("replaces object values wholesale (shallow, not deep merge)", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "c", { _id: "1", meta: { a: 1, b: 2 } });
    await store.update(scope, "c", { _id: "1" }, { meta: { a: 9 } });
    expect(await store.findOne(scope, "c", { _id: "1" })).toEqual({ _id: "1", meta: { a: 9 } });
  });
});

describe("delete and count", () => {
  it("deletes matching documents and reports the count", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    for (let i = 0; i < 5; i++) await store.insert(scope, "c", { _id: `d${i}`, i });
    expect(await store.delete(scope, "c", { i: { $gte: 3 } })).toEqual({ deleted: 2 });
    expect(await store.count(scope, "c", {})).toBe(3);
    expect(await store.count(scope, "c", { i: { $lt: 2 } })).toBe(2);
  });
});

describe("isolation and validation", () => {
  it("keeps collections apart", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await store.insert(scope, "a", { _id: "1", v: "a" });
    await store.insert(scope, "b", { _id: "1", v: "b" });
    expect((await store.findOne(scope, "a", { _id: "1" }))?.v).toBe("a");
    expect((await store.findOne(scope, "b", { _id: "1" }))?.v).toBe("b");
  });

  it("keeps scopes apart", async () => {
    const { store } = await makeStore();
    const one = freshScope();
    const two = freshScope();
    await store.insert(one, "c", { _id: "1", v: 1 });
    expect(await store.findOne(two, "c", { _id: "1" })).toBeNull();
  });

  it("rejects hostile collection names", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    for (const name of ['x"; DROP TABLE t; --', "", "a b", "ą", "x".repeat(65), '"x"']) {
      await expectCode(store.insert(scope, name, { v: 1 }), "INVALID_ARGUMENT");
    }
  });

  it("rejects unknown query operators (fail closed)", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await expectCode(store.find(scope, "c", { v: { $where: "1" } }), "INVALID_ARGUMENT");
    await expectCode(store.find(scope, "c", { $nor: [{ v: 1 }] }), "INVALID_ARGUMENT");
  });

  it("rejects oversized documents", async () => {
    const { store } = await makeStore();
    const scope = freshScope();
    await expectCode(
      store.insert(scope, "c", { blob: "x".repeat(5 * 1024 * 1024 + 1) }),
      "QUOTA_EXCEEDED",
    );
  });
});

describe("persistence", () => {
  it("documents survive a restart via the persist/load cycle", async () => {
    const persisted = new Map<string, Uint8Array>();
    const { store } = await makeStore({
      persist: (scope, bytes) => {
        persisted.set(scope.instanceId, bytes);
      },
    });
    const scope = freshScope();
    await store.insert(scope, "boards", { _id: "b1", title: "Todo", cards: ["one", "two"] });
    await store.update(scope, "boards", { _id: "b1" }, { title: "Doing" });

    const { engine } = await makeStore();
    // Flush the first engine, then hydrate a brand-new one from the bytes.
    const bytes = await (async () => {
      const first = persisted.get(scope.instanceId);
      expect(first).toBeDefined();
      return first as Uint8Array;
    })();
    const engine2 = await WaSqliteEngine.create(
      { wasmBinary },
      { functions: [docsRegexpFunction] },
    );
    const store2 = new DocsStore(engine2);
    const reopened = freshScope();
    engine2.load(reopened, bytes);
    void engine; // first engine's flush already happened via the map write
    const doc = await store2.findOne(reopened, "boards", { _id: "b1" });
    expect(doc).toEqual({ _id: "b1", title: "Doing", cards: ["one", "two"] });
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

  it("dispatches the full db.docs surface with session-derived scope", async () => {
    const { store } = await makeStore();
    const handler = docsBrokerHandler(store);
    const session = makeSession("docs-handler-instance", ["db.docs"]);

    const inserted = (await handler(session, "db.docs.insert", {
      name: "c",
      doc: { title: "x" },
    })) as { _id: string };
    expect(typeof inserted._id).toBe("string");
    const found = (await handler(session, "db.docs.find", {
      name: "c",
      query: { title: "x" },
    })) as unknown[];
    expect(found).toHaveLength(1);
    expect(await handler(session, "db.docs.count", { name: "c" })).toBe(1);
    expect(
      await handler(session, "db.docs.update", {
        name: "c",
        query: { title: "x" },
        patch: { done: true },
      }),
    ).toEqual({ matched: 1 });
    expect(await handler(session, "db.docs.delete", { name: "c", query: { done: true } })).toEqual({
      deleted: 1,
    });
  });

  it("refuses without the db.docs capability", async () => {
    const { store } = await makeStore();
    const handler = docsBrokerHandler(store);
    const session = makeSession("docs-nocap", ["db.kv"]);
    await expectCode(
      handler(session, "db.docs.find", { name: "c", query: {} }),
      "PERMISSION_DENIED",
    );
  });
});
