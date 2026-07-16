/**
 * The boundary broker: the single gate every privileged call passes through
 * (SPEC §7, SECURITY §4). Its invariants are executable, not aspirational:
 *
 * 1. **Scope is derived from the session (the window), never the message.**
 *    `instanceId`/`slot`/`connectionId` in a message are stripped by the guard
 *    and ignored here — a malicious app cannot address another POID's data.
 * 2. **Fail closed.** Unknown method or field → reject (in the guard).
 *    Ungranted capability → `PERMISSION_DENIED`.
 * 3. **No credential ever crosses the boundary**, in a result or an error.
 *    Errors are scrubbed to §9 codes and known secrets are redacted.
 * 4. **Every call is rate-limited and quota-checked.**
 *
 * In M04 the storage backend is in-memory and network/AI/MCP are injectable
 * handlers; a later milestone forwards these to Rust Core over IPC. The
 * boundary logic here does not change when that happens.
 */

import {
  type AppInfoDescriptor,
  type Capability,
  type FailureMessage,
  METHOD_CAPABILITY,
  type Method,
  PROTOCOL,
  type ResultMessage,
} from "@poid/sdk";
import type { DataEngine, Scope } from "./engine.js";
import { guardRequest } from "./guard.js";

/** A user-configured backend binding. The `secret` is held by the reader and
 * **never** leaves it (SPEC §7.1). */
export interface Connection {
  id: string;
  kind: "sql" | "net" | "ai" | "mcp";
  secret: string;
}

/** Everything the broker knows about one open POID window. Assembled by the
 * host from the container + user consent; the app cannot alter it. */
export interface ReaderSession {
  instanceId: string;
  appInfo: Omit<AppInfoDescriptor, "instanceId" | "slot">;
  capabilities: Set<Capability>;
  slots: string[];
  /** Reader-controlled active slot. The application cannot change this. */
  currentSlot: string;
  quotaBytes: number;
  connections: Map<string, Connection>;
}

/** Side-effecting backends the broker calls once a request is authorised.
 * Injected so M04 can run without a real network/AI stack; each receives the
 * session (for credentials the broker attaches) and returns only safe data. */
export interface BrokerHandlers {
  net?(session: ReaderSession, params: Record<string, unknown>): Promise<NetResult>;
  ai?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
  mcp?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
  files?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
  ui?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
  exportData?(
    session: ReaderSession,
    method: Method,
    params: Record<string, unknown>,
  ): Promise<unknown>;
  sql?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
}

interface NetResult {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  body: string;
}

interface RateLimit {
  capacity: number;
  refillPerSec: number;
}

interface BrokerOptions {
  handlers?: BrokerHandlers;
  rateLimit?: RateLimit;
  now?: () => number;
}

interface Bucket {
  tokens: number;
  last: number;
}

/** A generic reader-side failure that carries a safe §9 code. Handlers throw
 * this; anything else becomes a scrubbed `INTERNAL`. */
export class BrokerError extends Error {
  constructor(
    readonly code:
      | "PERMISSION_DENIED"
      | "QUOTA_EXCEEDED"
      | "NOT_AVAILABLE"
      | "CONNECTION_REQUIRED"
      | "INVALID_ARGUMENT"
      | "INTERNAL",
    message: string,
  ) {
    super(message);
    this.name = "BrokerError";
  }
}

export class Broker {
  private readonly engine: DataEngine;
  private readonly handlers: BrokerHandlers;
  private readonly rateLimit: RateLimit;
  private readonly now: () => number;
  private readonly sessions = new Map<string, ReaderSession>();
  private readonly buckets = new Map<string, Bucket>();
  /** Every secret the broker knows, redacted from any outgoing text. */
  private readonly secrets = new Set<string>();

  constructor(engine: DataEngine, options: BrokerOptions = {}) {
    this.engine = engine;
    this.handlers = options.handlers ?? {};
    this.rateLimit = options.rateLimit ?? { capacity: 120, refillPerSec: 60 };
    this.now = options.now ?? (() => Date.now());
  }

  /** Registers a window's session. `sessionId` is opaque and assigned by the
   * bridge from the window identity — never supplied by the app. */
  register(sessionId: string, session: ReaderSession): void {
    this.sessions.set(sessionId, session);
    this.buckets.set(sessionId, { tokens: this.rateLimit.capacity, last: this.now() });
    for (const conn of session.connections.values()) {
      if (conn.secret) this.secrets.add(conn.secret);
    }
  }

  /** Drops a window's session (on close). */
  unregister(sessionId: string): void {
    this.sessions.delete(sessionId);
    this.buckets.delete(sessionId);
  }

  /** Reader-only: switches the active slot. Not reachable from the sandbox —
   * there is no method that calls it (SPEC §6.4). */
  setSlot(sessionId: string, slot: string): void {
    const session = this.sessions.get(sessionId);
    if (session?.slots.includes(slot)) session.currentSlot = slot;
  }

  /**
   * Runs the full pipeline for one raw inbound message and returns the message
   * to post back to the sandbox, or `null` when the request cannot be
   * correlated (no usable id). Never throws.
   */
  async handle(sessionId: string, raw: unknown): Promise<ResultMessage | FailureMessage | null> {
    const session = this.sessions.get(sessionId);
    // An unregistered window is not addressable — drop, do not answer.
    if (!session) return null;

    const guarded = guardRequest(raw);
    if (!guarded.ok) {
      return guarded.id === null ? null : this.fail(guarded.id, guarded.code, guarded.message);
    }
    const { id, method, params } = guarded;

    if (!this.spend(sessionId)) {
      return this.fail(id, "QUOTA_EXCEEDED", "call rate limit exceeded");
    }

    const capability = METHOD_CAPABILITY[method];
    if (!session.capabilities.has(capability)) {
      return this.fail(id, "PERMISSION_DENIED", `capability \`${capability}\` not granted`);
    }

    try {
      const result = await this.dispatch(session, method, params);
      return { poid: PROTOCOL, id, ok: true, result };
    } catch (err) {
      if (err instanceof BrokerError) return this.fail(id, err.code, err.message);
      // Never surface an unexpected error's text: it may name host paths or
      // internals. Collapse to a generic INTERNAL.
      return this.fail(id, "INTERNAL", "internal reader error");
    }
  }

  private async dispatch(
    session: ReaderSession,
    method: Method,
    params: Record<string, unknown>,
  ): Promise<unknown> {
    const scope: Scope = { instanceId: session.instanceId, slot: session.currentSlot };
    switch (method) {
      case "app.info":
        return this.appInfo(session);
      case "app.setTitle":
        session.appInfo = { ...session.appInfo };
        return undefined;
      case "app.setDirty":
        return undefined;
      case "db.kv.get":
        return this.engine.kvGet(scope, str(params.key, "key"));
      case "db.kv.set": {
        const key = str(params.key, "key");
        this.checkKey(key);
        this.checkQuota(scope, key, params.value);
        this.engine.kvSet(scope, key, params.value);
        return undefined;
      }
      case "db.kv.delete":
        this.engine.kvDelete(scope, str(params.key, "key"));
        return undefined;
      case "db.kv.list":
        return this.engine.kvList(
          scope,
          params.prefix === undefined ? undefined : str(params.prefix, "prefix"),
        );
      case "db.kv.clear":
        this.engine.kvClear(scope);
        return undefined;
      case "db.slots.list":
        return session.slots.length > 0 ? [...session.slots] : ["default"];
      case "db.slots.current":
        return session.currentSlot || "default";
      case "net.fetch":
        return this.net(session, params);
      case "ai.complete":
      case "ai.models":
        return this.delegate(this.handlers.ai, session, method, params);
      case "mcp.servers":
      case "mcp.call":
        return this.delegate(this.handlers.mcp, session, method, params);
      case "files.open":
      case "files.save":
        return this.delegate(this.handlers.files, session, method, params);
      case "ui.notify":
      case "ui.confirm":
      case "ui.setBadge":
      case "ui.print":
      case "ui.clipboard.write":
      case "ui.clipboard.read":
        return this.delegate(this.handlers.ui, session, method, params);
      case "export.data":
      case "export.pdf":
        return this.delegate(this.handlers.exportData, session, method, params);
      case "db.sql.exec":
      case "db.sql.begin":
      case "db.sql.commit":
      case "db.sql.rollback":
        return this.delegate(this.handlers.sql, session, method, params);
      case "db.docs.insert":
      case "db.docs.find":
      case "db.docs.findOne":
      case "db.docs.update":
      case "db.docs.delete":
      case "db.docs.count":
        // The document store lands with the Data Engine milestone.
        throw new BrokerError("NOT_AVAILABLE", "document store not available in this reader build");
    }
  }

  private appInfo(session: ReaderSession): AppInfoDescriptor {
    return {
      ...session.appInfo,
      instanceId: session.instanceId,
      slot: session.currentSlot || null,
    };
  }

  private async net(session: ReaderSession, params: Record<string, unknown>): Promise<NetResult> {
    if (!this.handlers.net) {
      throw new BrokerError("NOT_AVAILABLE", "network is not available in this reader build");
    }
    // The broker performs the request and attaches credentials itself; whatever
    // the app put in `init.headers.authorization` was already dropped by the
    // SDK and is dropped again here.
    const init = (params.init ?? {}) as { headers?: Record<string, string> };
    if (init.headers) {
      for (const key of Object.keys(init.headers)) {
        if (key.toLowerCase() === "authorization") delete init.headers[key];
      }
    }
    return this.handlers.net(session, params);
  }

  private async delegate(
    handler:
      | ((s: ReaderSession, m: Method, p: Record<string, unknown>) => Promise<unknown>)
      | undefined,
    session: ReaderSession,
    method: Method,
    params: Record<string, unknown>,
  ): Promise<unknown> {
    if (!handler)
      throw new BrokerError("NOT_AVAILABLE", `\`${method}\` not available in this reader build`);
    return handler(session, method, params);
  }

  private spend(sessionId: string): boolean {
    const bucket = this.buckets.get(sessionId);
    if (!bucket) return false;
    const now = this.now();
    const elapsed = (now - bucket.last) / 1000;
    bucket.tokens = Math.min(
      this.rateLimit.capacity,
      bucket.tokens + elapsed * this.rateLimit.refillPerSec,
    );
    bucket.last = now;
    if (bucket.tokens < 1) return false;
    bucket.tokens -= 1;
    return true;
  }

  private checkKey(key: string): void {
    if (key.length > 256 || /[\s/\\'"]/.test(key)) {
      throw new BrokerError("INVALID_ARGUMENT", "invalid key");
    }
  }

  private checkQuota(scope: Scope, key: string, value: unknown): void {
    const size = key.length + JSON.stringify(value ?? null).length;
    if (size > 5 * 1024 * 1024) throw new BrokerError("QUOTA_EXCEEDED", "value exceeds 5 MB");
    const session = [...this.sessions.values()].find((s) => s.instanceId === scope.instanceId);
    const quota = session?.quotaBytes ?? Number.POSITIVE_INFINITY;
    if (this.engine.usage(scope) + size > quota) {
      throw new BrokerError("QUOTA_EXCEEDED", "storage quota exhausted");
    }
  }

  private fail(id: number, code: string, message: string): FailureMessage {
    return { poid: PROTOCOL, id, ok: false, error: { code, message: this.redact(message) } };
  }

  /** Defense in depth: strip any known secret from outgoing text, so a message
   * can never leak a credential even if a handler mishandles one. */
  private redact(message: string): string {
    let out = message;
    for (const secret of this.secrets) {
      if (secret.length >= 4 && out.includes(secret)) out = out.split(secret).join("***");
    }
    return out;
  }
}

function str(value: unknown, field: string): string {
  if (typeof value !== "string")
    throw new BrokerError("INVALID_ARGUMENT", `\`${field}\` must be a string`);
  return value;
}
