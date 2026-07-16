/**
 * Message-shape validation for the bridge. Fails closed: an unknown method or
 * an unexpected field is rejected, never guessed (SECURITY §4).
 *
 * Scope-identifying fields (`instanceId`, `slot`, `connectionId`) are a known
 * attack vector — an app trying to address another POID's data. They are
 * **stripped and ignored**, not treated as unknown fields, because the broker
 * derives scope from the sending window, never from the message.
 */

import { METHODS, type Method } from "@poid/sdk";

/** Fields an application must never be able to use to choose its own scope.
 * Stripped from every message before validation (SECURITY §4). */
export const SCOPE_FIELDS = ["instanceId", "slot", "connectionId"] as const;

interface ParamSpec {
  required: readonly string[];
  optional: readonly string[];
}

/** Allowed parameter keys per method. Anything outside this (after scope
 * fields are stripped) is an unknown field and the call is rejected. */
const PARAM_SPEC: Record<Method, ParamSpec> = {
  "app.info": { required: [], optional: [] },
  "app.setTitle": { required: ["title"], optional: [] },
  "app.setDirty": { required: ["dirty"], optional: [] },
  "db.kv.get": { required: ["key"], optional: [] },
  "db.kv.set": { required: ["key", "value"], optional: [] },
  "db.kv.delete": { required: ["key"], optional: [] },
  "db.kv.list": { required: [], optional: ["prefix"] },
  "db.kv.clear": { required: [], optional: [] },
  "db.sql.exec": { required: ["sql"], optional: ["params", "txId"] },
  "db.sql.begin": { required: [], optional: [] },
  "db.sql.commit": { required: ["txId"], optional: [] },
  "db.sql.rollback": { required: ["txId"], optional: [] },
  "db.docs.insert": { required: ["name", "doc"], optional: [] },
  "db.docs.find": { required: ["name"], optional: ["query", "opts"] },
  "db.docs.findOne": { required: ["name", "query"], optional: [] },
  "db.docs.update": { required: ["name", "query", "patch"], optional: [] },
  "db.docs.delete": { required: ["name", "query"], optional: [] },
  "db.docs.count": { required: ["name"], optional: ["query"] },
  "db.slots.list": { required: [], optional: [] },
  "db.slots.current": { required: [], optional: [] },
  "files.open": { required: [], optional: ["opts"] },
  "files.save": { required: ["blob", "suggestedName"], optional: [] },
  "net.fetch": { required: ["url"], optional: ["init"] },
  "ai.complete": { required: ["messages"], optional: ["model", "maxTokens", "stream"] },
  "ai.models": { required: [], optional: [] },
  "mcp.servers": { required: [], optional: [] },
  "mcp.call": { required: ["serverId", "tool", "args"], optional: [] },
  "ui.notify": { required: ["msg"], optional: ["level"] },
  "ui.confirm": { required: ["msg"], optional: [] },
  "ui.setBadge": { required: ["text"], optional: [] },
  "ui.print": { required: [], optional: [] },
  "ui.clipboard.write": { required: ["text"], optional: [] },
  "ui.clipboard.read": { required: [], optional: [] },
  "export.data": { required: ["payload"], optional: ["opts"] },
  "export.pdf": { required: [], optional: ["opts"] },
};

const METHOD_SET = new Set<string>(METHODS);

/** Outcome of validating a raw inbound message. */
export type GuardResult =
  | { ok: true; id: number; method: Method; params: Record<string, unknown> }
  | { ok: false; id: number | null; code: "INVALID_ARGUMENT" | "NOT_AVAILABLE"; message: string };

/** Validates a raw `postMessage` payload into a well-formed request, or a
 * typed rejection. Never throws. */
export function guardRequest(data: unknown): GuardResult {
  if (typeof data !== "object" || data === null) {
    return { ok: false, id: null, code: "INVALID_ARGUMENT", message: "message is not an object" };
  }
  const m = data as { poid?: unknown; id?: unknown; method?: unknown; params?: unknown };
  if (m.poid !== 1) {
    return { ok: false, id: null, code: "INVALID_ARGUMENT", message: "bad protocol marker" };
  }
  const id = typeof m.id === "number" && Number.isInteger(m.id) ? m.id : null;
  if (id === null) {
    return { ok: false, id: null, code: "INVALID_ARGUMENT", message: "missing request id" };
  }
  if (typeof m.method !== "string" || !METHOD_SET.has(m.method)) {
    return { ok: false, id, code: "NOT_AVAILABLE", message: "unknown method" };
  }
  const method = m.method as Method;

  const rawParams = m.params === undefined ? {} : m.params;
  if (typeof rawParams !== "object" || rawParams === null || Array.isArray(rawParams)) {
    return { ok: false, id, code: "INVALID_ARGUMENT", message: "params must be an object" };
  }

  // Strip scope-identifying fields: known attack surface, deliberately ignored.
  const params: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(rawParams)) {
    if ((SCOPE_FIELDS as readonly string[]).includes(key)) continue;
    params[key] = value;
  }

  const spec = PARAM_SPEC[method] ?? { required: [], optional: [] };
  const allowed = new Set<string>([...spec.required, ...spec.optional]);
  for (const key of Object.keys(params)) {
    if (!allowed.has(key)) {
      return { ok: false, id, code: "INVALID_ARGUMENT", message: `unexpected field \`${key}\`` };
    }
  }
  for (const key of spec.required) {
    if (!(key in params)) {
      return { ok: false, id, code: "INVALID_ARGUMENT", message: `missing field \`${key}\`` };
    }
  }

  return { ok: true, id, method, params };
}
