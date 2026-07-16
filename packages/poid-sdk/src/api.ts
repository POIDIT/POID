/**
 * Builds the `window.poid` object from the bootstrap capabilities and an RPC
 * client. Namespaces are attached **only** when their capability is present:
 * `poid.net`/`poid.ai`/`poid.mcp` are absent (not stubbed) unless granted, so
 * an application can feature-detect what it may do (RUNTIME-API §0).
 */

import type { AppInfoDescriptor, BootstrapConfig, Capability } from "./protocol.js";
import type { RpcClient } from "./rpc.js";

type Json = unknown;

/** The application-facing API surface (RUNTIME-API §1–§8). Optional namespaces
 * are present only when granted. */
export interface PoidAPI {
  app: {
    info(): Promise<AppInfoDescriptor>;
    onSuspend(cb: () => void | Promise<void>): void;
    setTitle(title: string): Promise<void>;
    setDirty(dirty: boolean): Promise<void>;
  };
  db: {
    kv: {
      get<T = Json>(key: string): Promise<T | null>;
      set<T = Json>(key: string, value: T): Promise<void>;
      delete(key: string): Promise<void>;
      list(prefix?: string): Promise<string[]>;
      clear(): Promise<void>;
    };
    sql?: {
      exec(
        sql: string,
        params?: unknown[],
      ): Promise<{ rows: Record<string, unknown>[]; rowsAffected: number }>;
      transaction(fn: (tx: SqlTx) => Promise<void>): Promise<void>;
    };
    docs: {
      collection(name: string): DocCollection;
    };
    slots: {
      list(): Promise<string[]>;
      current(): Promise<string>;
    };
  };
  files?: {
    open(opts?: { accept?: string[]; multiple?: boolean }): Promise<File[]>;
    save(blob: Blob, suggestedName: string): Promise<void>;
  };
  net?: {
    fetch(url: string, init?: RequestInit): Promise<Response>;
  };
  ai?: {
    complete(opts: AiCompleteOptions): Promise<AsyncIterable<string> | string>;
    models(): Promise<string[]>;
  };
  mcp?: {
    servers(): Promise<{ id: string; name: string; tools: string[] }[]>;
    call(serverId: string, tool: string, args: object): Promise<unknown>;
  };
  ui: {
    notify(msg: string, level?: "info" | "warn" | "error"): Promise<void>;
    confirm(msg: string): Promise<boolean>;
    setBadge(text: string | null): Promise<void>;
    print?(): Promise<void>;
    clipboard?: {
      write(text: string): Promise<void>;
      read(): Promise<string>;
    };
  };
  export: {
    data(payload: object, opts?: { schema?: string }): Promise<void>;
    pdf(opts?: { title?: string }): Promise<void>;
  };
}

interface SqlTx {
  exec(
    sql: string,
    params?: unknown[],
  ): Promise<{ rows: Record<string, unknown>[]; rowsAffected: number }>;
}
interface DocCollection {
  insert<T>(doc: T): Promise<{ _id: string }>;
  find<T>(query?: object, opts?: { limit?: number; skip?: number; sort?: object }): Promise<T[]>;
  findOne<T>(query: object): Promise<T | null>;
  update(query: object, patch: object): Promise<{ matched: number }>;
  delete(query: object): Promise<{ deleted: number }>;
  count(query?: object): Promise<number>;
}
interface AiCompleteOptions {
  messages: { role: "user" | "assistant" | "system"; content: string }[];
  model?: string;
  maxTokens?: number;
  stream?: boolean;
}

/** Builds `window.poid` for the granted capability set. */
export function buildPoidApi(config: BootstrapConfig, rpc: RpcClient): PoidAPI {
  const has = (cap: Capability): boolean => config.capabilities.includes(cap);
  const call = (method: string, params: unknown = {}): Promise<unknown> => rpc.call(method, params);

  const api: PoidAPI = {
    app: {
      info: () => call("app.info") as Promise<AppInfoDescriptor>,
      onSuspend: (cb) => rpc.on("suspend", () => void cb()),
      setTitle: (title) => call("app.setTitle", { title }) as Promise<void>,
      setDirty: (dirty) => call("app.setDirty", { dirty }) as Promise<void>,
    },
    db: {
      kv: {
        get: <T>(key: string) => call("db.kv.get", { key }) as Promise<T | null>,
        set: <T>(key: string, value: T) => call("db.kv.set", { key, value }) as Promise<void>,
        delete: (key) => call("db.kv.delete", { key }) as Promise<void>,
        list: (prefix) => call("db.kv.list", { prefix }) as Promise<string[]>,
        clear: () => call("db.kv.clear") as Promise<void>,
      },
      docs: {
        collection: (name) => makeCollection(call, name),
      },
      slots: {
        list: () => call("db.slots.list") as Promise<string[]>,
        current: () => call("db.slots.current") as Promise<string>,
      },
    },
    ui: {
      notify: (msg, level) => call("ui.notify", { msg, level }) as Promise<void>,
      confirm: (msg) => call("ui.confirm", { msg }) as Promise<boolean>,
      setBadge: (text) => call("ui.setBadge", { text }) as Promise<void>,
    },
    export: {
      data: (payload, opts) => call("export.data", { payload, opts }) as Promise<void>,
      pdf: (opts) => call("export.pdf", { opts }) as Promise<void>,
    },
  };

  if (has("db.sql")) {
    api.db.sql = {
      exec: (sql, params) =>
        call("db.sql.exec", { sql, params }) as Promise<{
          rows: Record<string, unknown>[];
          rowsAffected: number;
        }>,
      transaction: async (fn) => {
        const txId = (await call("db.sql.begin")) as string;
        try {
          await fn({
            exec: (sql, params) =>
              call("db.sql.exec", { sql, params, txId }) as Promise<{
                rows: Record<string, unknown>[];
                rowsAffected: number;
              }>,
          });
          await call("db.sql.commit", { txId });
        } catch (err) {
          await call("db.sql.rollback", { txId });
          throw err;
        }
      },
    };
  }

  if (has("files")) {
    api.files = {
      open: (opts) => call("files.open", { opts }) as Promise<File[]>,
      save: (blob, suggestedName) => call("files.save", { blob, suggestedName }) as Promise<void>,
    };
  }

  if (has("net")) {
    api.net = {
      fetch: async (url, init) => {
        const r = (await call("net.fetch", { url, init: serializableInit(init) })) as {
          status: number;
          statusText: string;
          headers: Record<string, string>;
          body: string;
        };
        return new Response(r.body, {
          status: r.status,
          statusText: r.statusText,
          headers: r.headers,
        });
      },
    };
  }

  if (has("ai")) {
    api.ai = {
      complete: (opts) => call("ai.complete", opts) as Promise<string>,
      models: () => call("ai.models") as Promise<string[]>,
    };
  }

  if (has("mcp")) {
    api.mcp = {
      servers: () =>
        call("mcp.servers") as Promise<{ id: string; name: string; tools: string[] }[]>,
      call: (serverId, tool, args) => call("mcp.call", { serverId, tool, args }),
    };
  }

  if (has("ui.print")) {
    api.ui.print = () => call("ui.print") as Promise<void>;
  }
  if (has("ui.clipboard")) {
    api.ui.clipboard = {
      write: (text) => call("ui.clipboard.write", { text }) as Promise<void>,
      read: () => call("ui.clipboard.read") as Promise<string>,
    };
  }

  return api;
}

function makeCollection(
  call: (m: string, p?: unknown) => Promise<unknown>,
  name: string,
): DocCollection {
  return {
    insert: <T>(doc: T) => call("db.docs.insert", { name, doc }) as Promise<{ _id: string }>,
    find: <T>(query?: object, opts?: object) =>
      call("db.docs.find", { name, query, opts }) as Promise<T[]>,
    findOne: <T>(query: object) => call("db.docs.findOne", { name, query }) as Promise<T | null>,
    update: (query, patch) =>
      call("db.docs.update", { name, query, patch }) as Promise<{ matched: number }>,
    delete: (query) => call("db.docs.delete", { name, query }) as Promise<{ deleted: number }>,
    count: (query) => call("db.docs.count", { name, query }) as Promise<number>,
  };
}

/** Reduces a `RequestInit` to a structured-cloneable shape. Any `Authorization`
 * header the app tries to set is dropped here as well as at the broker — the
 * broker attaches credentials, the app never supplies them (RUNTIME-API §4). */
function serializableInit(init?: RequestInit): unknown {
  if (!init) return undefined;
  const headers: Record<string, string> = {};
  const src = init.headers;
  const entries: [string, string][] =
    src instanceof Headers
      ? [...src.entries()]
      : Array.isArray(src)
        ? (src as [string, string][])
        : src
          ? Object.entries(src as Record<string, string>)
          : [];
  for (const [key, value] of entries) {
    if (key.toLowerCase() !== "authorization") headers[key] = String(value);
  }
  return {
    method: init.method,
    headers,
    body: typeof init.body === "string" ? init.body : undefined,
  };
}
