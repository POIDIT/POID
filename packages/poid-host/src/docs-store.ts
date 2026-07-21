/**
 * `poid.db.docs` — the document tier of the Data Engine (SPEC §8,
 * RUNTIME-API §2.3): collections stored over the SQL engine, one table per
 * collection (`_id TEXT PRIMARY KEY, doc TEXT`), queried through the
 * compiled Mongo-like subset ({@link compileQuery}).
 *
 * This layer is shared by every platform: it holds no storage of its own and
 * no scope logic of its own — it composes SQL for {@link WaSqliteEngine},
 * which owns isolation, quota, and persistence. Collection names are the
 * only interpolated identifier and pass a strict allowlist; everything else
 * is a bound parameter.
 *
 * `update(query, patch)` is a **shallow merge** (Mongo `$set` semantics):
 * each top-level patch key replaces that field, other fields survive.
 * Documents are plain JSON objects; `_id` is a string (caller-supplied or a
 * generated UUID) and lives both in the doc and in the indexed column.
 */

import type { Method } from "@poid/sdk";
import { BrokerError, type ReaderSession } from "./broker.js";
import { compileQuery, compileSort, fieldPath } from "./docs-query.js";
import type { Scope } from "./engine.js";
import {
  type ScalarFunction,
  type ScopeResolver,
  type SqlCallOptions,
  sessionScope,
  type WaSqliteEngine,
} from "./sql-engine.js";

/** Collection names: short, filesystem-ish, nothing an identifier quoter
 * could stumble over. The table name is `docs::<name>`, double-quoted. */
const COLLECTION_NAME = /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}$/;

/** Per-document size cap — mirrors the kv value cap (RUNTIME-API §2.1). */
const MAX_DOC_BYTES = 5 * 1024 * 1024;

/** The `regexp(pattern, value)` scalar function the query compiler emits.
 * Register it on the engine (`functions: [docsRegexpFunction]`) wherever the
 * document store is wired. Patterns are compiled once per store lifetime. */
export const docsRegexpFunction: ScalarFunction = (() => {
  const cache = new Map<string, RegExp | null>();
  return {
    name: "regexp",
    nArg: 2,
    fn: ([pattern, value]: unknown[]) => {
      if (typeof pattern !== "string" || typeof value !== "string") return null;
      let re = cache.get(pattern);
      if (re === undefined) {
        try {
          re = new RegExp(pattern);
        } catch {
          re = null;
        }
        if (cache.size > 256) cache.clear();
        cache.set(pattern, re);
      }
      return re ? re.test(value) : null;
    },
  };
})();

/** Find options (RUNTIME-API §2.3). */
export interface FindOptions {
  limit?: number;
  skip?: number;
  sort?: unknown;
}

export class DocsStore {
  private readonly engine: WaSqliteEngine;

  constructor(engine: WaSqliteEngine) {
    this.engine = engine;
  }

  /** Inserts a document; returns its `_id`. */
  async insert(
    scope: Scope,
    name: string,
    doc: unknown,
    call: SqlCallOptions = {},
  ): Promise<{ _id: string }> {
    const table = this.table(name);
    if (typeof doc !== "object" || doc === null || Array.isArray(doc)) {
      throw new BrokerError("INVALID_ARGUMENT", "a document must be a plain object");
    }
    const record = { ...(doc as Record<string, unknown>) };
    const id = record._id === undefined ? crypto.randomUUID() : record._id;
    if (typeof id !== "string" || id.length === 0 || id.length > 128) {
      throw new BrokerError("INVALID_ARGUMENT", "`_id` must be a non-empty string (max 128)");
    }
    record._id = id;
    const json = encodeDoc(record);
    await this.ensure(scope, table, call);
    try {
      await this.engine.exec(
        scope,
        `INSERT INTO ${table} (_id, doc) VALUES (?, ?)`,
        [id, json],
        call,
      );
    } catch (err) {
      if (err instanceof BrokerError && /UNIQUE/i.test(err.message)) {
        throw new BrokerError("INVALID_ARGUMENT", `a document with _id \`${id}\` already exists`);
      }
      throw err;
    }
    return { _id: id };
  }

  /** Finds documents matching `query`, with sort/skip/limit. */
  async find(
    scope: Scope,
    name: string,
    query: unknown,
    opts: FindOptions = {},
    call: SqlCallOptions = {},
  ): Promise<Record<string, unknown>[]> {
    const table = this.table(name);
    await this.ensure(scope, table, call);
    const where = compileQuery(query);
    const order = compileSort(opts.sort);
    const limit = clampCount(opts.limit, -1, "limit");
    const skip = clampCount(opts.skip, 0, "skip");
    const { rows } = await this.engine.exec(
      scope,
      `SELECT doc FROM ${table} WHERE ${where.sql} ORDER BY ${order.sql} LIMIT ? OFFSET ?`,
      [...where.params, ...order.params, limit, skip],
      call,
    );
    return rows.map((row) => JSON.parse(row.doc as string) as Record<string, unknown>);
  }

  /** First match or null. */
  async findOne(
    scope: Scope,
    name: string,
    query: unknown,
    call: SqlCallOptions = {},
  ): Promise<Record<string, unknown> | null> {
    const docs = await this.find(scope, name, query, { limit: 1 }, call);
    return docs[0] ?? null;
  }

  /** Shallow-merges `patch` into every matching document. */
  async update(
    scope: Scope,
    name: string,
    query: unknown,
    patch: unknown,
    call: SqlCallOptions = {},
  ): Promise<{ matched: number }> {
    const table = this.table(name);
    if (typeof patch !== "object" || patch === null || Array.isArray(patch)) {
      throw new BrokerError("INVALID_ARGUMENT", "patch must be a plain object");
    }
    const entries = Object.entries(patch as Record<string, unknown>);
    for (const [key] of entries) {
      if (key.startsWith("$")) {
        throw new BrokerError(
          "INVALID_ARGUMENT",
          "patch is a shallow merge; update operators like `$set` are not supported",
        );
      }
    }
    if (entries.some(([key]) => key === "_id")) {
      throw new BrokerError("INVALID_ARGUMENT", "`_id` cannot be changed");
    }
    await this.ensure(scope, table, call);
    const where = compileQuery(query);
    if (entries.length === 0) {
      const { rows } = await this.engine.exec(
        scope,
        `SELECT COUNT(*) AS n FROM ${table} WHERE ${where.sql}`,
        [...where.params],
        call,
      );
      return { matched: (rows[0]?.n as number) ?? 0 };
    }
    // One statement: json_set(doc, '$."key"', json(?), ...) — a shallow merge
    // executed by SQLite, atomic across all matched documents. Dotted keys
    // address nested fields (json_set does not create missing parents).
    const setters: string[] = [];
    const params: unknown[] = [];
    for (const [key, value] of entries) {
      setters.push("?, json(?)");
      params.push(fieldPath(key), encodeDoc(value));
    }
    const result = await this.engine.exec(
      scope,
      `UPDATE ${table} SET doc = json_set(doc, ${setters.join(", ")}) WHERE ${where.sql}`,
      [...params, ...where.params],
      call,
    );
    return { matched: result.rowsAffected };
  }

  /** Deletes matching documents. */
  async delete(
    scope: Scope,
    name: string,
    query: unknown,
    call: SqlCallOptions = {},
  ): Promise<{ deleted: number }> {
    const table = this.table(name);
    await this.ensure(scope, table, call);
    const where = compileQuery(query);
    const result = await this.engine.exec(
      scope,
      `DELETE FROM ${table} WHERE ${where.sql}`,
      [...where.params],
      call,
    );
    return { deleted: result.rowsAffected };
  }

  /** Counts matching documents. */
  async count(
    scope: Scope,
    name: string,
    query: unknown,
    call: SqlCallOptions = {},
  ): Promise<number> {
    const table = this.table(name);
    await this.ensure(scope, table, call);
    const where = compileQuery(query);
    const { rows } = await this.engine.exec(
      scope,
      `SELECT COUNT(*) AS n FROM ${table} WHERE ${where.sql}`,
      [...where.params],
      call,
    );
    return (rows[0]?.n as number) ?? 0;
  }

  private table(name: string): string {
    if (typeof name !== "string" || !COLLECTION_NAME.test(name)) {
      throw new BrokerError(
        "INVALID_ARGUMENT",
        "collection names are 1-64 chars of letters, digits, `_`, `.`, `-`",
      );
    }
    return `"docs::${name}"`;
  }

  private async ensure(scope: Scope, table: string, call: SqlCallOptions): Promise<void> {
    await this.engine.exec(
      scope,
      `CREATE TABLE IF NOT EXISTS ${table} (_id TEXT PRIMARY KEY, doc TEXT NOT NULL)`,
      undefined,
      call,
    );
  }
}

/** Serialises a JSON value with the doc-size cap; rejects non-JSON input. */
function encodeDoc(value: unknown): string {
  let json: string;
  try {
    json = JSON.stringify(value);
  } catch {
    throw new BrokerError("INVALID_ARGUMENT", "document is not JSON-serialisable");
  }
  if (json === undefined) {
    throw new BrokerError("INVALID_ARGUMENT", "document is not JSON-serialisable");
  }
  if (json.length > MAX_DOC_BYTES) {
    throw new BrokerError("QUOTA_EXCEEDED", "document exceeds 5 MB");
  }
  return json;
}

function clampCount(value: unknown, fallback: number, field: string): number {
  if (value === undefined) return fallback;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new BrokerError("INVALID_ARGUMENT", `\`${field}\` must be a non-negative integer`);
  }
  return value;
}

// ------------------------------------------------------------------ broker

const DOCS_CAPABILITY = "db.docs";

/**
 * Adapts a {@link DocsStore} to the broker's injectable `docs` handler.
 * Scope comes from the session (the window) via `scopeFor`, never the
 * message; the reader picks the resolver from `storage.mode`.
 */
export function docsBrokerHandler(
  store: DocsStore,
  scopeFor: ScopeResolver = sessionScope,
): (session: ReaderSession, method: Method, params: Record<string, unknown>) => Promise<unknown> {
  return async (session, method, params) => {
    if (!session.capabilities.has(DOCS_CAPABILITY)) {
      throw new BrokerError("PERMISSION_DENIED", `capability \`${DOCS_CAPABILITY}\` not granted`);
    }
    const scope: Scope = scopeFor(session);
    const call: SqlCallOptions = { quotaBytes: session.quotaBytes };
    const name = params.name as string;
    switch (method) {
      case "db.docs.insert":
        return store.insert(scope, name, params.doc, call);
      case "db.docs.find":
        return store.find(scope, name, params.query, findOptions(params.opts), call);
      case "db.docs.findOne":
        return store.findOne(scope, name, params.query, call);
      case "db.docs.update":
        return store.update(scope, name, params.query, params.patch, call);
      case "db.docs.delete":
        return store.delete(scope, name, params.query, call);
      case "db.docs.count":
        return store.count(scope, name, params.query, call);
      default:
        throw new BrokerError("NOT_AVAILABLE", `unexpected docs method \`${method}\``);
    }
  };
}

function findOptions(value: unknown): FindOptions {
  if (value === undefined) return {};
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new BrokerError("INVALID_ARGUMENT", "`opts` must be an object");
  }
  const raw = value as Record<string, unknown>;
  const opts: FindOptions = {};
  if (raw.limit !== undefined) opts.limit = raw.limit as number;
  if (raw.skip !== undefined) opts.skip = raw.skip as number;
  if (raw.sort !== undefined) opts.sort = raw.sort;
  return opts;
}
