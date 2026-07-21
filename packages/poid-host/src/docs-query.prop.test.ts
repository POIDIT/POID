/**
 * Property tier for the docs query compiler (CONVENTIONS: attacker-shaped
 * code paths get property tests). A corpus of generated documents is
 * inserted once; then, for arbitrary queries from the operator grammar, the
 * SQL result set must equal a reference JavaScript evaluator implementing
 * the documented semantics — including the deliberate divergences from
 * SQLite's own habits (typed booleans, missing-vs-null, `$ne` matching
 * missing fields).
 */

import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import fc from "fast-check";
import { beforeAll, describe, expect, it } from "vitest";
import { DocsStore, docsRegexpFunction } from "./docs-store.js";
import type { Scope } from "./engine.js";
import { WaSqliteEngine } from "./sql-engine.js";

const require = createRequire(import.meta.url);

// ------------------------------------------------------------- reference

type Scalar = string | number | boolean | null;

function getPath(doc: unknown, field: string): unknown {
  let current: unknown = doc;
  for (const segment of field.split(".")) {
    if (current === null || typeof current !== "object") return undefined;
    current = (current as Record<string, unknown>)[
      Array.isArray(current) ? (Number(segment) as unknown as string) : segment
    ];
  }
  return current;
}

function refEq(value: unknown, expected: Scalar): boolean {
  if (expected === null) return value === null || value === undefined;
  if (typeof expected === "boolean") return value === expected;
  if (typeof expected === "number") return typeof value === "number" && value === expected;
  return value === expected;
}

function refOrdered(value: unknown, op: string, expected: number | string): boolean {
  if (typeof expected === "number" && typeof value !== "number") return false;
  if (typeof expected === "string" && typeof value !== "string") return false;
  const v = value as number | string;
  switch (op) {
    case "$gt":
      return v > expected;
    case "$gte":
      return v >= expected;
    case "$lt":
      return v < expected;
    default:
      return v <= expected;
  }
}

function refField(doc: unknown, field: string, spec: unknown): boolean {
  const value = getPath(doc, field);
  if (
    spec === null ||
    typeof spec === "string" ||
    typeof spec === "number" ||
    typeof spec === "boolean"
  ) {
    return refEq(value, spec);
  }
  const ops = spec as Record<string, unknown>;
  return Object.entries(ops).every(([op, arg]) => {
    switch (op) {
      case "$eq":
        return refEq(value, arg as Scalar);
      case "$ne":
        return !refEq(value, arg as Scalar);
      case "$gt":
      case "$gte":
      case "$lt":
      case "$lte":
        return refOrdered(value, op, arg as number | string);
      case "$in":
        return (arg as Scalar[]).some((x) => refEq(value, x));
      case "$nin":
        return !(arg as Scalar[]).some((x) => refEq(value, x));
      case "$exists":
        return arg ? value !== undefined : value === undefined;
      case "$regex":
        return typeof value === "string" && new RegExp(arg as string).test(value);
      default:
        throw new Error(`reference evaluator: unknown op ${op}`);
    }
  });
}

function refQuery(doc: unknown, query: Record<string, unknown>): boolean {
  return Object.entries(query).every(([key, value]) => {
    if (key === "$and") return (value as Record<string, unknown>[]).every((q) => refQuery(doc, q));
    if (key === "$or") return (value as Record<string, unknown>[]).some((q) => refQuery(doc, q));
    return refField(doc, key, value);
  });
}

// ------------------------------------------------------------ generators

const scalarPool: Scalar[] = [
  0,
  1,
  -1,
  2.5,
  1000,
  Number.MAX_SAFE_INTEGER,
  "",
  "a",
  "b",
  "kanban-42",
  "'; DROP TABLE x; --",
  'quo"te',
  "back\\slash",
  "żółć",
  true,
  false,
  null,
];

const scalarArb = fc.constantFrom(...scalarPool);
const fieldArb = fc.constantFrom("a", "b", "c", "nested.x", "tags.0");

const fieldSpecArb: fc.Arbitrary<unknown> = fc.oneof(
  scalarArb,
  fc.record({ $eq: scalarArb }, { requiredKeys: ["$eq"] }),
  fc.record({ $ne: scalarArb }, { requiredKeys: ["$ne"] }),
  fc
    .tuple(
      fc.constantFrom("$gt", "$gte", "$lt", "$lte"),
      fc.oneof(fc.constantFrom<number | string>(0, 1, 2.5, 1000), fc.constantFrom("a", "b", "m")),
    )
    .map(([op, v]) => ({ [op]: v })),
  fc.array(scalarArb, { maxLength: 3 }).map((list) => ({ $in: list })),
  fc.array(scalarArb, { maxLength: 3 }).map((list) => ({ $nin: list })),
  fc.boolean().map((b) => ({ $exists: b })),
  fc.constantFrom("^a", "kanban-\\d+", "^$", "b$").map((p) => ({ $regex: p })),
);

const leafQueryArb: fc.Arbitrary<Record<string, unknown>> = fc
  .tuple(fieldArb, fieldSpecArb)
  .map(([field, spec]) => ({ [field]: spec }));

const queryArb: fc.Arbitrary<Record<string, unknown>> = fc.oneof(
  { depthSize: "small" },
  leafQueryArb,
  fc
    .tuple(fc.constantFrom("$and", "$or"), fc.array(leafQueryArb, { minLength: 1, maxLength: 3 }))
    .map(([op, list]) => ({ [op]: list })),
  fc.tuple(leafQueryArb, leafQueryArb).map(([a, b]) => ({ ...a, ...b })),
);

const docArb = fc.record(
  {
    a: scalarArb,
    b: scalarArb,
    c: scalarArb,
    nested: fc.record({ x: scalarArb }, { requiredKeys: [] }),
    tags: fc.array(scalarArb, { maxLength: 3 }),
  },
  { requiredKeys: [] },
);

// ---------------------------------------------------------------- corpus

let store: DocsStore;
let scope: Scope;
let corpus: Record<string, unknown>[];

beforeAll(async () => {
  const bytes = await readFile(require.resolve("wa-sqlite/dist/wa-sqlite.wasm"));
  const wasmBinary = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(wasmBinary).set(bytes);
  const engine = await WaSqliteEngine.create({ wasmBinary }, { functions: [docsRegexpFunction] });
  store = new DocsStore(engine);
  scope = { instanceId: "prop-instance", slot: "" };

  corpus = fc.sample(docArb, { seed: 42, numRuns: 80 }).map((doc, i) => ({
    ...doc,
    _id: `doc-${String(i).padStart(3, "0")}`,
  }));
  for (const doc of corpus) await store.insert(scope, "corpus", doc);
});

describe("query compiler vs reference evaluator", () => {
  it("find() returns exactly the documents the reference semantics select", async () => {
    await fc.assert(
      fc.asyncProperty(queryArb, async (query) => {
        const found = await store.find(scope, "corpus", query, { sort: { _id: 1 } });
        const foundIds = found.map((d) => d._id);
        const expected = corpus
          .filter((doc) => refQuery(doc, query))
          .map((d) => d._id)
          .sort();
        expect(foundIds).toEqual(expected);
      }),
      { numRuns: 150 },
    );
  });

  it("count() agrees with find()", async () => {
    await fc.assert(
      fc.asyncProperty(queryArb, async (query) => {
        const found = await store.find(scope, "corpus", query);
        const counted = await store.count(scope, "corpus", query);
        expect(counted).toBe(found.length);
      }),
      { numRuns: 50 },
    );
  });
});
