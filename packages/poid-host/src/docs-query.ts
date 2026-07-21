/**
 * The `poid.db.docs` query compiler: the Mongo-like subset (RUNTIME-API
 * §2.3 — `$eq $ne $gt $gte $lt $lte $in $nin $and $or $exists $regex`)
 * compiled to a SQLite WHERE clause over `json_extract(doc, ?)`.
 *
 * Every piece of application data — field paths and compared values alike —
 * is **bound as a parameter**, never interpolated into SQL text. The only
 * interpolated fragments are operators and literal type-name lists from this
 * file. An unknown operator or an untranslatable value is rejected with
 * `INVALID_ARGUMENT` (fail closed), never guessed.
 *
 * Type fidelity follows Mongo where SQLite would blur it: JSON booleans
 * extract as 0/1, so boolean comparisons are guarded with
 * `json_type IN ('true','false')` and numeric ones with
 * `json_type IN ('integer','real')` — `{done: true}` does not match an
 * integer 1 and `{n: 1}` does not match `true`. Equality uses `IS`, so
 * `{f: null}` matches both JSON null and a missing field (as in Mongo), and
 * `$ne`/`$nin` match missing fields (as in Mongo).
 */

import { BrokerError } from "./broker.js";

/** A compiled WHERE fragment: SQL with `?` placeholders and its bindings. */
export interface CompiledWhere {
  sql: string;
  params: unknown[];
}

/** Scalars a query may compare against. */
type QueryScalar = string | number | boolean | null;

const COMPARATORS = { $gt: ">", $gte: ">=", $lt: "<", $lte: "<=" } as const;

function invalid(message: string): never {
  throw new BrokerError("INVALID_ARGUMENT", message);
}

/**
 * Translates a Mongo dot-path (`boards.0.title`) into a bindable SQLite JSON
 * path (`$."boards"[0]."title"`). Rejects segments that would escape the
 * quoted form; the result is always passed as a bound parameter.
 */
export function fieldPath(field: string): string {
  if (field.length === 0) invalid("empty field name");
  if (field.startsWith("$")) invalid(`field \`${field}\` must not start with $`);
  const segments = field.split(".");
  let path = "$";
  for (const segment of segments) {
    if (segment.length === 0) invalid(`field \`${field}\` has an empty path segment`);
    for (const ch of segment) {
      // Control characters, quotes and backslashes would escape the quoted
      // JSON-path form; nothing legitimate needs them in a field name.
      if (ch.charCodeAt(0) < 0x20 || ch === '"' || ch === "\\") {
        invalid(`field \`${field}\` contains characters not allowed in a path`);
      }
    }
    path += /^\d+$/.test(segment) ? `[${segment}]` : `."${segment}"`;
  }
  return path;
}

function isQueryScalar(value: unknown): value is QueryScalar {
  return (
    value === null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  );
}

/** Equality with Mongo semantics; every other predicate composes from it. */
function eqCondition(path: string, value: QueryScalar, params: unknown[]): string {
  if (value === null) {
    params.push(path);
    return `json_extract(doc, ?) IS NULL`;
  }
  if (typeof value === "boolean") {
    params.push(path, path, value ? 1 : 0);
    return `(json_type(doc, ?) IN ('true','false') AND json_extract(doc, ?) IS ?)`;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) invalid("non-finite numbers cannot be queried");
    params.push(path, path, value);
    return `(json_type(doc, ?) IN ('integer','real') AND json_extract(doc, ?) IS ?)`;
  }
  params.push(path, value);
  return `json_extract(doc, ?) IS ?`;
}

function orderedCondition(
  path: string,
  op: keyof typeof COMPARATORS,
  value: unknown,
  params: unknown[],
): string {
  if (typeof value === "number") {
    if (!Number.isFinite(value)) invalid("non-finite numbers cannot be queried");
    params.push(path, path, value);
    return `(json_type(doc, ?) IN ('integer','real') AND json_extract(doc, ?) ${COMPARATORS[op]} ?)`;
  }
  if (typeof value === "string") {
    params.push(path, path, value);
    return `(json_type(doc, ?) = 'text' AND json_extract(doc, ?) ${COMPARATORS[op]} ?)`;
  }
  invalid(`\`${op}\` compares numbers or strings`);
}

function scalarList(op: string, value: unknown): QueryScalar[] {
  if (!Array.isArray(value)) invalid(`\`${op}\` takes an array`);
  for (const item of value) {
    if (!isQueryScalar(item)) invalid(`\`${op}\` takes scalars (string/number/boolean/null)`);
  }
  return value as QueryScalar[];
}

/** One field's predicate: a bare scalar (implicit $eq) or an operator object. */
function fieldCondition(field: string, spec: unknown, params: unknown[]): string {
  const path = fieldPath(field);
  if (isQueryScalar(spec)) return eqCondition(path, spec, params);
  if (typeof spec !== "object" || Array.isArray(spec)) {
    invalid(`field \`${field}\` must be compared to a scalar or an operator object`);
  }
  const entries = Object.entries(spec as Record<string, unknown>);
  if (entries.length === 0) invalid(`field \`${field}\` has an empty operator object`);
  const parts: string[] = [];
  for (const [op, value] of entries) {
    switch (op) {
      case "$eq":
        if (!isQueryScalar(value)) invalid("`$eq` compares scalars");
        parts.push(eqCondition(path, value, params));
        break;
      case "$ne":
        if (!isQueryScalar(value)) invalid("`$ne` compares scalars");
        parts.push(`NOT ${eqCondition(path, value, params)}`);
        break;
      case "$gt":
      case "$gte":
      case "$lt":
      case "$lte":
        parts.push(orderedCondition(path, op, value, params));
        break;
      case "$in": {
        const list = scalarList("$in", value);
        if (list.length === 0) {
          parts.push("0");
          break;
        }
        parts.push(`(${list.map((v) => eqCondition(path, v, params)).join(" OR ")})`);
        break;
      }
      case "$nin": {
        const list = scalarList("$nin", value);
        if (list.length === 0) {
          parts.push("1");
          break;
        }
        parts.push(`NOT (${list.map((v) => eqCondition(path, v, params)).join(" OR ")})`);
        break;
      }
      case "$exists":
        if (typeof value !== "boolean") invalid("`$exists` takes a boolean");
        params.push(path);
        parts.push(`json_type(doc, ?) IS ${value ? "NOT NULL" : "NULL"}`);
        break;
      case "$regex": {
        if (typeof value !== "string") invalid("`$regex` takes a string pattern");
        try {
          // Compile eagerly: a bad pattern is the caller's error, not a
          // silent NULL from the UDF at row-match time.
          new RegExp(value);
        } catch {
          invalid("`$regex` pattern does not compile");
        }
        params.push(path, value, path);
        parts.push(`(json_type(doc, ?) = 'text' AND regexp(?, json_extract(doc, ?)))`);
        break;
      }
      default:
        invalid(`unknown operator \`${op}\``);
    }
  }
  return parts.length === 1 ? (parts[0] as string) : `(${parts.join(" AND ")})`;
}

function clauseList(op: string, value: unknown, params: unknown[]): string {
  if (!Array.isArray(value) || value.length === 0) {
    invalid(`\`${op}\` takes a non-empty array of queries`);
  }
  const parts = value.map((q) => compileCondition(q, params));
  return `(${parts.join(op === "$and" ? " AND " : " OR ")})`;
}

function compileCondition(query: unknown, params: unknown[]): string {
  if (query === undefined || query === null) return "1";
  if (typeof query !== "object" || Array.isArray(query)) invalid("query must be an object");
  const entries = Object.entries(query as Record<string, unknown>);
  if (entries.length === 0) return "1";
  const parts: string[] = [];
  for (const [key, value] of entries) {
    if (key === "$and" || key === "$or") {
      parts.push(clauseList(key, value, params));
    } else if (key.startsWith("$")) {
      invalid(`unknown operator \`${key}\``);
    } else {
      parts.push(fieldCondition(key, value, params));
    }
  }
  return parts.length === 1 ? (parts[0] as string) : `(${parts.join(" AND ")})`;
}

/** Compiles a query object into a WHERE fragment. */
export function compileQuery(query: unknown): CompiledWhere {
  const params: unknown[] = [];
  const sql = compileCondition(query, params);
  return { sql, params };
}

/** Compiles a Mongo-style sort (`{field: 1 | -1, ...}`) into ORDER BY terms.
 * `_id` is always appended as the tiebreaker so ordering is deterministic. */
export function compileSort(sort: unknown): CompiledWhere {
  if (sort === undefined) return { sql: "_id", params: [] };
  if (typeof sort !== "object" || sort === null || Array.isArray(sort)) {
    invalid("sort must be an object of field: 1 | -1");
  }
  const params: unknown[] = [];
  const terms: string[] = [];
  for (const [field, direction] of Object.entries(sort as Record<string, unknown>)) {
    if (direction !== 1 && direction !== -1) invalid("sort direction must be 1 or -1");
    params.push(fieldPath(field));
    terms.push(`json_extract(doc, ?) ${direction === 1 ? "ASC" : "DESC"}`);
  }
  terms.push("_id");
  return { sql: terms.join(", "), params };
}
