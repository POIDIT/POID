/**
 * Platform wiring for the SQL tier: lazy engine construction plus the
 * persistence port every reader implements once.
 *
 * The WASM engine (~550 KB) must not sit in any reader's startup path, so
 * the broker handlers built here create it on the **first** `poid.db.sql` /
 * `poid.db.docs` call, seed the calling scope from persisted bytes, and only
 * then delegate. Each platform supplies two things: where the WASM lives
 * ({@link SqliteWasmSource}) and where database bytes go
 * ({@link SqlPersistence}) — the desktop Reader persists to the Rust vault
 * over IPC, the Web Reader (and desktop embedded mode) to IndexedDB.
 */

import type { BrokerHandlers } from "./broker.js";
import { DocsStore, docsBrokerHandler, docsRegexpFunction } from "./docs-store.js";
import type { Scope } from "./engine.js";
import { applyMigrations, type Migration, stampSchemaVersion } from "./migrations.js";
import { dumpSql } from "./sql-dump.js";
import {
  type ScalarFunction,
  type ScopeResolver,
  type SqlEngineOptions,
  sessionScope,
  sqlBrokerHandler,
  WaSqliteEngine,
} from "./sql-engine.js";
import type { SqliteWasmSource } from "./sql-wasm-api.js";

/** Where a platform keeps SQL database bytes, addressed by scope. */
export interface SqlPersistence {
  load(scope: Scope): Promise<Uint8Array | undefined>;
  persist(scope: Scope, bytes: Uint8Array): void | Promise<void>;
}

/** Options for {@link makeSqlHandlers}. */
export interface SqlHandlersOptions {
  wasm: SqliteWasmSource;
  /** Omit for a purely in-memory session (consent previews, tests). */
  persistence?: SqlPersistence;
  /**
   * The container's embedded `data/database.sql` (UTF-8 SQL text), executed
   * into a scope that has no persisted bytes yet — the SQL counterpart of
   * `seedFromStore`. Runs lazily on the scope's first call, which is after
   * consent: a declined app never executes its seed.
   */
  seedSql?: Uint8Array;
  /**
   * How to derive the storage scope from a session. Defaults to the window's
   * own instance + slot; a connection (M10.5) overrides this to key storage
   * by the connection, so every POID configured against it shares a database.
   * Chosen by the reader, never by the app.
   */
  scopeFor?: ScopeResolver;
  /**
   * The manifest's `storage.schema_version` (SPEC §12). A fresh scope is
   * stamped at this version; an existing scope below it is migrated up to it.
   * Default 0 — apps without a schema version and without migrations are
   * completely unaffected.
   */
  schemaVersion?: number;
  /** The container's ordered `migrations/` scripts (see {@link parseMigrations}). */
  migrations?: Migration[];
  /** Extra engine tuning; `regexp` for the document store is always added. */
  engineOptions?: Omit<SqlEngineOptions, "persist">;
}

/** The wired handlers plus the reader-side lifecycle hooks. */
export interface SqlHandlers {
  sql: NonNullable<BrokerHandlers["sql"]>;
  docs: NonNullable<BrokerHandlers["docs"]>;
  /** Awaits write-behind persists. A no-op when the engine never woke up. */
  flush(): Promise<void>;
  /**
   * Committed database bytes for `scope` (export / download paths), or null
   * when the scope has no SQL data at all. Falls back to persisted bytes
   * when the engine was never created this session.
   */
  snapshot(scope: Scope): Promise<Uint8Array | null>;
  /**
   * The scope's canonical SQL text dump (`data/database.sql` form), or null
   * when it holds no user objects. Wakes the engine if needed.
   */
  dump(scope: Scope): Promise<string | null>;
}

function partition(scope: Scope): string {
  return JSON.stringify([scope.instanceId, scope.slot]);
}

/** Builds the lazy `sql` + `docs` broker handlers for one reader window. */
export function makeSqlHandlers(options: SqlHandlersOptions): SqlHandlers {
  const scopeFor = options.scopeFor ?? sessionScope;
  let ready: Promise<{
    engine: WaSqliteEngine;
    sql: SqlHandlers["sql"];
    docs: SqlHandlers["docs"];
  }> | null = null;
  const seeded = new Map<string, Promise<void>>();

  function wake() {
    if (!ready) {
      ready = (async () => {
        const functions: ScalarFunction[] = [
          docsRegexpFunction,
          ...(options.engineOptions?.functions ?? []),
        ];
        const engine = await WaSqliteEngine.create(options.wasm, {
          ...options.engineOptions,
          functions,
          persist: options.persistence
            ? (scope, bytes) => options.persistence?.persist(scope, bytes)
            : undefined,
        });
        return {
          engine,
          sql: sqlBrokerHandler(engine, scopeFor),
          docs: docsBrokerHandler(new DocsStore(engine), scopeFor),
        };
      })();
    }
    return ready;
  }

  /**
   * Prepares `scope` exactly once, before its first call: loads persisted
   * bytes (or seeds a fresh scope from the embedded dump), then reconciles
   * its schema version — stamp a fresh database at the app's current
   * version, migrate an older one up to it (SPEC §12).
   */
  function seedScope(engine: WaSqliteEngine, scope: Scope): Promise<void> {
    const key = partition(scope);
    let pending = seeded.get(key);
    if (!pending) {
      pending = (async () => {
        const bytes = await options.persistence?.load(scope);
        const fresh = !(bytes && bytes.byteLength > 0);
        if (!fresh) {
          engine.load(scope, bytes as Uint8Array);
        } else if (options.seedSql && options.seedSql.byteLength > 0) {
          await engine.exec(scope, new TextDecoder().decode(options.seedSql));
        }

        const schemaVersion = options.schemaVersion ?? 0;
        const migrations = options.migrations ?? [];
        if (schemaVersion > 0 || migrations.length > 0) {
          if (fresh) {
            // A new install is already at the current schema (built by the
            // app's own code / the seed); mark it so, don't migrate an empty
            // database.
            await stampSchemaVersion(engine, scope, schemaVersion);
          } else {
            // Existing data: bridge it up to the app's current schema.
            await applyMigrations(engine, scope, schemaVersion, migrations);
          }
        }
      })();
      seeded.set(key, pending);
    }
    return pending;
  }

  return {
    sql: async (session, method, params) => {
      const wired = await wake();
      await seedScope(wired.engine, scopeFor(session));
      return wired.sql(session, method, params);
    },
    docs: async (session, method, params) => {
      const wired = await wake();
      await seedScope(wired.engine, scopeFor(session));
      return wired.docs(session, method, params);
    },
    flush: async () => {
      if (!ready) return;
      const { engine } = await ready;
      await engine.flush();
    },
    snapshot: async (scope) => {
      if (ready) {
        const { engine } = await ready;
        await seedScope(engine, scope);
        const bytes = await engine.snapshot(scope);
        return bytes.byteLength > 0 ? bytes : null;
      }
      const persisted = await options.persistence?.load(scope);
      return persisted && persisted.byteLength > 0 ? persisted : null;
    },
    dump: async (scope) => {
      // Don't load the ~550 KB engine just to dump nothing: if it never woke
      // this session and the scope has no persisted bytes either, there is no
      // SQL data at all. (This keeps the download path of a kv-only app from
      // fetching wa-sqlite.wasm.)
      if (!ready) {
        const persisted = await options.persistence?.load(scope);
        if (!persisted || persisted.byteLength === 0) return null;
      }
      const { engine } = await wake();
      await seedScope(engine, scope);
      return dumpSql(engine, scope);
    },
  };
}

// ------------------------------------------------------------- IndexedDB

const IDB_STORE = "sql";

/**
 * IndexedDB-backed {@link SqlPersistence}: one record per (instanceId,
 * slot), keyed as an array. Used by the Web Reader (all modes) and by the
 * desktop Reader's embedded mode — the same place their kv tiers live.
 */
export class IdbSqlPersistence implements SqlPersistence {
  private readonly db: IDBDatabase;

  private constructor(db: IDBDatabase) {
    this.db = db;
  }

  static open(name: string): Promise<IdbSqlPersistence> {
    return new Promise((resolve, reject) => {
      const req = indexedDB.open(name, 1);
      req.onupgradeneeded = () => {
        req.result.createObjectStore(IDB_STORE);
      };
      req.onsuccess = () => resolve(new IdbSqlPersistence(req.result));
      req.onerror = () => reject(req.error ?? new Error("IndexedDB open failed"));
    });
  }

  load(scope: Scope): Promise<Uint8Array | undefined> {
    return new Promise((resolve, reject) => {
      const req = this.db
        .transaction(IDB_STORE, "readonly")
        .objectStore(IDB_STORE)
        .get([scope.instanceId, scope.slot]);
      req.onsuccess = () => resolve(req.result as Uint8Array | undefined);
      req.onerror = () => reject(req.error ?? new Error("IndexedDB read failed"));
    });
  }

  persist(scope: Scope, bytes: Uint8Array): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(IDB_STORE, "readwrite");
      tx.objectStore(IDB_STORE).put(bytes, [scope.instanceId, scope.slot]);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("IndexedDB write failed"));
    });
  }

  /** Removes a scope's bytes (consent rollback, duplicate-as-empty). */
  remove(scope: Scope): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(IDB_STORE, "readwrite");
      tx.objectStore(IDB_STORE).delete([scope.instanceId, scope.slot]);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("IndexedDB delete failed"));
    });
  }
}
