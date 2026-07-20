import "fake-indexeddb/auto";
import { describe, expect, it } from "vitest";
import type { Scope } from "./engine.js";
import { IndexedDbEngine } from "./idb-engine.js";

const scopeA: Scope = { instanceId: "aaaaaaaa-1111-4111-8111-111111111111", slot: "" };
const scopeB: Scope = { instanceId: "bbbbbbbb-2222-4222-8222-222222222222", slot: "" };

let dbCounter = 0;
function freshName(): string {
  return `poid-web-test-${++dbCounter}`;
}

describe("IndexedDbEngine", () => {
  it("round-trips kv values with copy semantics", async () => {
    const engine = await IndexedDbEngine.open(freshName());
    engine.kvSet(scopeA, "todos", [{ id: 1, done: false }]);
    const value = engine.kvGet(scopeA, "todos") as { id: number; done: boolean }[];
    expect(value).toEqual([{ id: 1, done: false }]);
    const first = value[0];
    if (first) first.done = true;
    expect(engine.kvGet(scopeA, "todos")).toEqual([{ id: 1, done: false }]);
    engine.close();
  });

  it("partitions scopes: two instances never see each other's data", async () => {
    const engine = await IndexedDbEngine.open(freshName());
    engine.kvSet(scopeA, "note", "from A");
    engine.kvSet(scopeB, "note", "from B");
    expect(engine.kvGet(scopeA, "note")).toBe("from A");
    expect(engine.kvGet(scopeB, "note")).toBe("from B");
    engine.kvClear(scopeA);
    expect(engine.kvGet(scopeA, "note")).toBeNull();
    expect(engine.kvGet(scopeB, "note")).toBe("from B");
    engine.close();
  });

  it("persists across close and reopen", async () => {
    const name = freshName();
    const first = await IndexedDbEngine.open(name);
    first.kvSet(scopeA, "count", 42);
    first.kvDelete(scopeA, "never-was");
    await first.flush();
    first.close();

    const second = await IndexedDbEngine.open(name);
    expect(second.kvGet(scopeA, "count")).toBeNull();
    await second.hydrate(scopeA);
    expect(second.kvGet(scopeA, "count")).toBe(42);
    second.close();
  });

  it("seeds from an embedded store without overwriting existing keys", async () => {
    const engine = await IndexedDbEngine.open(freshName());
    engine.kvSet(scopeA, "kept", "mine");
    engine.seed(scopeA, { kept: "from-file", added: "from-file" });
    expect(engine.kvGet(scopeA, "kept")).toBe("mine");
    expect(engine.kvGet(scopeA, "added")).toBe("from-file");
    engine.close();
  });

  it("snapshots a scope as a store.json-shaped object", async () => {
    const engine = await IndexedDbEngine.open(freshName());
    expect(engine.isEmpty(scopeA)).toBe(true);
    engine.kvSet(scopeA, "b", 2);
    engine.kvSet(scopeA, "a", 1);
    expect(engine.isEmpty(scopeA)).toBe(false);
    expect(engine.snapshot(scopeA)).toEqual({ a: 1, b: 2 });
    expect(Object.keys(engine.snapshot(scopeA))).toEqual(["a", "b"]);
    engine.close();
  });

  it("lists and clears like the reference in-memory engine", async () => {
    const engine = await IndexedDbEngine.open(freshName());
    engine.kvSet(scopeA, "todo:1", "x");
    engine.kvSet(scopeA, "todo:2", "y");
    engine.kvSet(scopeA, "other", "z");
    expect(engine.kvList(scopeA, "todo:")).toEqual(["todo:1", "todo:2"]);
    expect(engine.kvList(scopeA)).toEqual(["other", "todo:1", "todo:2"]);
    expect(engine.usage(scopeA)).toBeGreaterThan(0);
    engine.kvClear(scopeA);
    expect(engine.kvList(scopeA)).toEqual([]);
    expect(engine.usage(scopeA)).toBe(0);
    engine.close();
  });
});
