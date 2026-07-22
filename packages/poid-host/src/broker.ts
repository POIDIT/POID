/**
 * The boundary broker: the single gate every privileged call passes through
 * (SPEC §7, SECURITY §4). Its invariants are executable, not aspirational:
 *
 * 1. **Scope is derived from the session (the window), never the message.**
 *    `instanceId`/`slot`/`connectionId` in a message are stripped by the guard
 *    and ignored here — a malicious app cannot address another POID's data.
 * 2. **Fail closed.** Unknown method or field → reject (in the guard).
 *    Ungranted capability → `PERMISSION_DENIED`.
 * 3. **No credential ever crosses the boundary**, in a result or an error —
 *    and, since M11, no credential is on this side of it either. The broker
 *    holds no secret and has no connection field to hold one in (SPEC §7.1).
 * 4. **Text that crosses is reader-authored.** A failure carries a §9 code and
 *    a constant string; the real reason goes to the host's diagnostics. Text
 *    assembled from a backend's response can carry a credential, and text that
 *    cannot vary cannot carry anything (SPEC §7.2.4).
 * 5. **Every call is rate-limited and quota-checked.**
 *
 * The storage backend and the network/AI/MCP handlers are injected, so this
 * file is the same whether they are in-memory test doubles or IPC calls into
 * Rust Core. The boundary logic does not change when the backend does.
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

/**
 * Everything the broker knows about one open POID window. Assembled by the
 * host from the container + user consent; the app cannot alter it.
 *
 * **There is no connection here, and no credential.** Through M10 this carried
 * a `connections: Map<string, Connection>` whose entries held a `secret`,
 * which meant every configured credential sat in the Reader window's
 * JavaScript heap — the same browser-engine process that hosts the
 * application's iframe. SPEC §7.1 now forbids that, so the field is gone: the
 * broker authorises an operation and Core performs it with the credential
 * attached on the far side.
 *
 * Which connection serves a window is likewise absent, because the application
 * must not be able to find out (SPEC §7.2.4). The reader expresses the binding
 * by choosing *which handlers it installs*, not by telling the broker.
 */
export interface ReaderSession {
  instanceId: string;
  appInfo: Omit<AppInfoDescriptor, "instanceId" | "slot">;
  capabilities: Set<Capability>;
  slots: string[];
  /** Reader-controlled active slot. The application cannot change this. */
  currentSlot: string;
  quotaBytes: number;
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
  docs?(session: ReaderSession, method: Method, params: Record<string, unknown>): Promise<unknown>;
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

/** A refusal, as the host may record it for the user. */
export interface Diagnostic {
  method: Method | null;
  code: string;
  /** The real reason, which does **not** cross to the application. */
  detail: string;
}

interface BrokerOptions {
  handlers?: BrokerHandlers;
  rateLimit?: RateLimit;
  now?: () => number;
  /**
   * Where the detail of a refusal goes, since it no longer crosses the
   * boundary. Studio routes this to the log the *user* can read — a person
   * debugging their own database deserves the real message, and the
   * application does not (SPEC §7.2.4).
   */
  onDiagnostic?: (entry: Diagnostic) => void;
}

/**
 * What an application is told, per error code.
 *
 * Every value is a constant. That is the mechanism, not the styling: text
 * assembled from a backend's response can carry a credential, and text that
 * cannot vary cannot carry anything. The code is the contract
 * (`RUNTIME-API.md` §9); the detail goes to {@link Diagnostic} instead.
 */
const INTERNAL_MESSAGE = "something went wrong inside the reader";

const ERROR_MESSAGE: Record<string, string> = {
  PERMISSION_DENIED: "this application has not been granted that",
  QUOTA_EXCEEDED: "this application has used all the space it is allowed",
  NOT_AVAILABLE: "this reader cannot do that",
  CONNECTION_REQUIRED: "this application needs a connection that is not set up",
  INVALID_ARGUMENT: "the request was not valid",
  INTERNAL: INTERNAL_MESSAGE,
};

interface Bucket {
  tokens: number;
  last: number;
}

/**
 * A reader-side failure carrying a §9 code. Handlers throw this; anything else
 * becomes an `INTERNAL`.
 *
 * The `message` is **host-side only**. It reaches {@link Diagnostic} and never
 * the application, so a handler may put the real reason in it — including
 * whatever a backend said — without that becoming a disclosure.
 */
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
  private readonly onDiagnostic: ((entry: Diagnostic) => void) | undefined;

  constructor(engine: DataEngine, options: BrokerOptions = {}) {
    this.engine = engine;
    this.handlers = options.handlers ?? {};
    this.rateLimit = options.rateLimit ?? { capacity: 120, refillPerSec: 60 };
    this.now = options.now ?? (() => Date.now());
    this.onDiagnostic = options.onDiagnostic;
  }

  /** Registers a window's session. `sessionId` is opaque and assigned by the
   * bridge from the window identity — never supplied by the app. */
  register(sessionId: string, session: ReaderSession): void {
    this.sessions.set(sessionId, session);
    this.buckets.set(sessionId, { tokens: this.rateLimit.capacity, last: this.now() });
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
      return guarded.id === null
        ? null
        : this.fail(guarded.id, guarded.code, null, guarded.message);
    }
    const { id, method, params } = guarded;

    if (!this.spend(sessionId)) {
      return this.fail(id, "QUOTA_EXCEEDED", method, "call rate limit exceeded");
    }

    const capability = METHOD_CAPABILITY[method];
    if (!session.capabilities.has(capability)) {
      return this.fail(id, "PERMISSION_DENIED", method, `capability \`${capability}\` not granted`);
    }

    try {
      const result = await this.dispatch(session, method, params);
      return { poid: PROTOCOL, id, ok: true, result };
    } catch (err) {
      // A handler's own text stays on this side of the boundary: it may quote
      // a backend, and a backend may quote its connection string. The code
      // crosses, the reason goes to the host's diagnostics (SPEC §7.2.4).
      if (err instanceof BrokerError) return this.fail(id, err.code, method, err.message);
      return this.fail(id, "INTERNAL", method, err instanceof Error ? err.message : String(err));
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
        return this.delegate(this.handlers.docs, session, method, params);
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

  /**
   * Builds the refusal that crosses to the application, and routes the real
   * reason to the host instead.
   *
   * `detail` never appears in the returned message. There is no argument that
   * would make it, which is the point: the only way to leak a backend's text
   * from here would be to change this function, and that is a change a
   * reviewer will see.
   */
  private fail(id: number, code: string, method: Method | null, detail: string): FailureMessage {
    this.onDiagnostic?.({ method, code, detail });
    return {
      poid: PROTOCOL,
      id,
      ok: false,
      error: { code, message: ERROR_MESSAGE[code] ?? INTERNAL_MESSAGE },
    };
  }
}

function str(value: unknown, field: string): string {
  if (typeof value !== "string")
    throw new BrokerError("INVALID_ARGUMENT", `\`${field}\` must be a string`);
  return value;
}
