/**
 * `poid.db.sql` — the SQL tier of the Data Engine (SPEC §8, RUNTIME-API
 * §2.2): SQLite compiled to WASM (wa-sqlite), one database per (instanceId,
 * slot) scope, each inside its own private in-memory VFS ({@link ScopeVfs}).
 *
 * **Isolation is structural, not lexical.** The engine never inspects SQL
 * text: a scope's VFS contains only that scope's database, so no name an
 * application can utter — including via `ATTACH DATABASE` — reaches another
 * scope. (Extension loading is compiled out of wa-sqlite; there is no
 * filesystem behind the VFS at all.)
 *
 * **Concurrency.** One SQLite connection per scope; every operation runs
 * under a per-scope mutex. A transaction (RUNTIME-API §2.2) holds a logical
 * lock: operations not carrying its token wait until it ends, so a stray
 * `exec` can never observe — or commit — someone else's half-done
 * transaction. An idle transaction is rolled back after a timeout, so an
 * abandoned token cannot wedge the scope. Manual `BEGIN`/`COMMIT` inside
 * `exec` text is rejected deterministically: transactions exist only through
 * the API, which is what keeps persistence snapshots consistent.
 *
 * **Persistence** is write-behind like every engine here: when a call leaves
 * the database both changed and committed, its bytes are snapshotted and
 * handed to the injected `persist` callback on a single promise chain;
 * `flush()` awaits the chain and rethrows the first failure.
 *
 * **Quota** is enforced by SQLite itself: `PRAGMA max_page_count` is derived
 * from the manifest's `storage.quota_mb`, and `SQLITE_FULL` surfaces as
 * `QUOTA_EXCEEDED` (RUNTIME-API §9).
 */

import type { Capability, Method } from "@poid/sdk";
import { SQLiteError } from "wa-sqlite/src/sqlite-api.js";
import { BrokerError, type ReaderSession } from "./broker.js";
import type { Scope } from "./engine.js";
import { ScopeVfs } from "./sql-vfs.js";
import {
  type DbHandle,
  loadSqlite,
  SQLITE_FULL,
  SQLITE_OPEN_CREATE,
  SQLITE_OPEN_READWRITE,
  SQLITE_ROW,
  SQLITE_UTF8,
  type SqliteApi,
  type SqliteWasmSource,
  type StmtHandle,
} from "./sql-wasm-api.js";

/** The database file name inside each scope's private VFS. */
const MAIN_DB = "main.db";

/** One result set (RUNTIME-API §2.2). */
export interface SqlResult {
  rows: Record<string, unknown>[];
  rowsAffected: number;
}

/** A scalar SQL function the reader registers on every scope's connection
 * (e.g. `regexp` for the document store). Must be pure and synchronous. */
export interface ScalarFunction {
  name: string;
  nArg: number;
  fn: (args: unknown[]) => unknown;
}

/** Tuning and wiring for {@link WaSqliteEngine}. */
export interface SqlEngineOptions {
  /** Called write-behind with the database bytes after a committed change. */
  persist?: (scope: Scope, bytes: Uint8Array) => void | Promise<void>;
  /** Reader-registered scalar functions (see {@link ScalarFunction}). */
  functions?: ScalarFunction[];
  /** Roll back a transaction idle for this long. Default 30 000 ms. */
  txIdleTimeoutMs?: number;
  /** Result caps per call; paging is the app's job. */
  maxResultRows?: number;
  maxResultBytes?: number;
  /** Default per-scope quota when a call does not carry one (SPEC §3.1). */
  defaultQuotaBytes?: number;
}

/** Per-call options threaded from the broker session. */
export interface SqlCallOptions {
  txId?: string;
  quotaBytes?: number;
}

interface TxState {
  id: string;
  closed: Promise<void>;
  close: () => void;
  timer: ReturnType<typeof setTimeout> | null;
}

interface ScopeState {
  db: DbHandle;
  vfs: ScopeVfs;
  /** Statement-level mutex: tail of the operation chain. */
  chain: Promise<void>;
  tx: TxState | null;
  /** `vfs.writes` at the last scheduled persist — the dirty check. */
  persistedWrites: number;
  persistQueued: boolean;
  /** Set under the mutex by {@link WaSqliteEngine.closeScope}; late persists
   * and timers check it instead of touching a closed connection. */
  closed: boolean;
}

/** Serialises a scope into a map key (JSON — unambiguous, no separator). */
function partition(scope: Scope): string {
  return JSON.stringify([scope.instanceId, scope.slot]);
}

/** Sentinel for the retry loop in the transaction gate. */
const RETRY = Symbol("retry");

export class WaSqliteEngine {
  private readonly sqlite3: SqliteApi;
  private readonly options: SqlEngineOptions;
  private readonly states = new Map<string, Promise<ScopeState>>();
  private readonly preloads = new Map<string, Uint8Array>();
  private persistChain: Promise<void> = Promise.resolve();
  private persistFailure: unknown = null;
  private vfsSeq = 0;

  private constructor(sqlite3: SqliteApi, options: SqlEngineOptions) {
    this.sqlite3 = sqlite3;
    this.options = options;
  }

  /** Loads wa-sqlite from `source` and builds an engine. */
  static async create(
    source: SqliteWasmSource,
    options: SqlEngineOptions = {},
  ): Promise<WaSqliteEngine> {
    return new WaSqliteEngine(await loadSqlite(source), options);
  }

  /**
   * Seeds a scope's database bytes (embedded-mode open, vault hydration).
   * Must be called before the scope's first statement — seeding an open
   * scope is a host wiring bug, not an application-reachable state.
   */
  load(scope: Scope, bytes: Uint8Array): void {
    const key = partition(scope);
    if (this.states.has(key)) {
      throw new BrokerError("INTERNAL", "cannot seed an open SQL scope");
    }
    this.preloads.set(key, bytes.slice());
  }

  /** Runs `sql` (optionally with `params` bound to a single statement). */
  async exec(
    scope: Scope,
    sql: string,
    params?: unknown[],
    call: SqlCallOptions = {},
  ): Promise<SqlResult> {
    const state = await this.scopeState(scope, call.quotaBytes);
    return this.gated(state, call.txId, async () => {
      const before = await this.totalChanges(state.db);
      let result: SqlResult;
      try {
        result = await this.run(state.db, sql, params);
      } catch (err) {
        // A failed statement can make SQLite roll the transaction back on
        // its own; reconcile our transaction state before rethrowing.
        this.reconcileTx(state);
        this.schedulePersist(scope, state);
        throw toBrokerError(err);
      }
      result.rowsAffected = (await this.totalChanges(state.db)) - before;
      this.enforceTxDiscipline(state, call.txId);
      this.schedulePersist(scope, state);
      return result;
    });
  }

  /** Opens a transaction and returns its token (RUNTIME-API §2.2). */
  async begin(scope: Scope, call: SqlCallOptions = {}): Promise<string> {
    const state = await this.scopeState(scope, call.quotaBytes);
    return this.gated(state, undefined, async () => {
      await this.sqlite3.exec(state.db, "BEGIN");
      const id = crypto.randomUUID();
      let close!: () => void;
      const closed = new Promise<void>((resolve) => {
        close = resolve;
      });
      state.tx = { id, closed, close, timer: null };
      this.touchTx(state);
      return id;
    });
  }

  /** Commits the transaction identified by `txId`. */
  async commit(scope: Scope, txId: string): Promise<void> {
    const state = await this.scopeState(scope, undefined);
    return this.gated(state, txId, async () => {
      if (this.sqlite3.get_autocommit(state.db) !== 0) {
        // SQLite already ended the transaction (auto-rollback on error).
        this.closeTx(state);
        throw new BrokerError(
          "INVALID_ARGUMENT",
          "the transaction was already rolled back after an error",
        );
      }
      try {
        await this.sqlite3.exec(state.db, "COMMIT");
      } catch (err) {
        this.reconcileTx(state);
        throw toBrokerError(err);
      }
      this.closeTx(state);
      this.schedulePersist(scope, state);
    });
  }

  /** Rolls back the transaction identified by `txId`. */
  async rollback(scope: Scope, txId: string): Promise<void> {
    const state = await this.scopeState(scope, undefined);
    return this.gated(state, txId, async () => {
      if (this.sqlite3.get_autocommit(state.db) === 0) {
        await this.sqlite3.exec(state.db, "ROLLBACK");
      }
      this.closeTx(state);
      this.schedulePersist(scope, state);
    });
  }

  /** Copies the scope's current committed database bytes (export paths).
   * Waits for an open transaction to end first. */
  async snapshot(scope: Scope): Promise<Uint8Array> {
    const state = await this.scopeState(scope, undefined);
    return this.gated(state, undefined, async () => {
      return state.vfs.snapshot(MAIN_DB) ?? new Uint8Array();
    });
  }

  /** Current database size in bytes (0 when the scope was never opened). */
  async usage(scope: Scope): Promise<number> {
    const key = partition(scope);
    const pending = this.states.get(key);
    if (!pending) return this.preloads.get(key)?.byteLength ?? 0;
    const state = await pending;
    return state.vfs.fileSize(MAIN_DB);
  }

  /** Awaits every scheduled persist; rethrows the first failure. */
  async flush(): Promise<void> {
    await this.persistChain;
    if (this.persistFailure !== null) {
      const failure = this.persistFailure;
      this.persistFailure = null;
      throw failure instanceof Error ? failure : new Error(String(failure));
    }
  }

  /** Rolls back any open transaction and closes the scope's connection. */
  async closeScope(scope: Scope): Promise<void> {
    const key = partition(scope);
    const pending = this.states.get(key);
    if (!pending) return;
    this.states.delete(key);
    const state = await pending;
    await this.gated(state, state.tx?.id, async () => {
      if (this.sqlite3.get_autocommit(state.db) === 0) {
        await this.sqlite3.exec(state.db, "ROLLBACK");
      }
      this.closeTx(state);
      state.closed = true;
      await this.sqlite3.close(state.db);
    });
  }

  // ---------------------------------------------------------------- scopes

  private scopeState(scope: Scope, quotaBytes: number | undefined): Promise<ScopeState> {
    const key = partition(scope);
    let pending = this.states.get(key);
    if (!pending) {
      pending = this.openScope(key, quotaBytes);
      this.states.set(key, pending);
      // A failed open must not poison the scope forever.
      pending.catch(() => {
        if (this.states.get(key) === pending) this.states.delete(key);
      });
    }
    return pending;
  }

  private async openScope(key: string, quotaBytes: number | undefined): Promise<ScopeState> {
    const vfs = new ScopeVfs(`poid-sql-${this.vfsSeq++}`);
    const preload = this.preloads.get(key);
    if (preload) {
      vfs.load(MAIN_DB, preload);
      this.preloads.delete(key);
    }
    this.sqlite3.vfs_register(vfs, false);
    const db = await this.sqlite3.open_v2(
      MAIN_DB,
      SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE,
      vfs.name,
    );
    // MEMORY journal: commit/rollback never create journal files, so the
    // main file is the whole persistent state and snapshots are consistent
    // whenever autocommit is on. Foreign keys default ON: POID apps target a
    // fresh standard, not SQLite's backward-compatibility default.
    await this.sqlite3.exec(db, "PRAGMA journal_mode=MEMORY");
    await this.sqlite3.exec(db, "PRAGMA foreign_keys=ON");
    const quota = quotaBytes ?? this.options.defaultQuotaBytes ?? 64 * 1024 * 1024;
    const pageSize = asNumber(await this.scalar(db, "PRAGMA page_size")) || 4096;
    const pages = Math.max(1, Math.floor(quota / pageSize));
    await this.sqlite3.exec(db, `PRAGMA max_page_count=${pages}`);
    for (const f of this.options.functions ?? []) this.registerFunction(db, f);
    return {
      db,
      vfs,
      chain: Promise.resolve(),
      tx: null,
      persistedWrites: vfs.writes,
      persistQueued: false,
      closed: false,
    };
  }

  private registerFunction(db: DbHandle, f: ScalarFunction): void {
    this.sqlite3.create_function(db, f.name, f.nArg, SQLITE_UTF8, 0, (context, values) => {
      // A UDF callback runs on the WASM stack: it must never throw. Any
      // failure or unsupported return degrades to SQL NULL.
      let out: unknown;
      try {
        // `values` is a typed array of value pointers — never .map() it, a
        // typed array's map coerces every result back to a number.
        const args: unknown[] = [];
        for (let i = 0; i < values.length; i++) {
          const value = this.sqlite3.value(values[i] as number);
          args.push(value instanceof Uint8Array ? new Uint8Array(value) : value);
        }
        out = f.fn(args);
      } catch {
        out = null;
      }
      if (typeof out === "boolean") out = out ? 1 : 0;
      if (
        out === null ||
        typeof out === "number" ||
        typeof out === "string" ||
        typeof out === "bigint" ||
        out instanceof Uint8Array
      ) {
        this.sqlite3.result(context, out);
      } else {
        this.sqlite3.result(context, null);
      }
    });
  }

  // --------------------------------------------------------- concurrency

  /** Runs `op` under the scope's statement mutex. */
  private async locked<T>(state: ScopeState, op: () => Promise<T>): Promise<T> {
    const previous = state.chain;
    let release!: () => void;
    state.chain = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await op();
    } finally {
      release();
    }
  }

  /**
   * The transaction gate. With a token: validate it and run inside the
   * transaction. Without: wait until no transaction is open, then take the
   * mutex — re-checking under it, because a `begin` may have slipped in
   * while we waited.
   */
  private async gated<T>(
    state: ScopeState,
    txId: string | undefined,
    op: () => Promise<T>,
  ): Promise<T> {
    if (txId !== undefined) {
      return this.locked(state, async () => {
        if (state.tx?.id !== txId) {
          throw new BrokerError(
            "INVALID_ARGUMENT",
            "no such transaction (an idle transaction is rolled back automatically)",
          );
        }
        this.touchTx(state);
        return op();
      });
    }
    for (;;) {
      while (state.tx) await state.tx.closed;
      const outcome = await this.locked(state, async (): Promise<{ value: T } | typeof RETRY> => {
        if (state.tx) return RETRY;
        return { value: await op() };
      });
      if (outcome !== RETRY) return outcome.value;
    }
  }

  private touchTx(state: ScopeState): void {
    const tx = state.tx;
    if (!tx) return;
    if (tx.timer) clearTimeout(tx.timer);
    const timeout = this.options.txIdleTimeoutMs ?? 30_000;
    tx.timer = setTimeout(() => {
      void this.expireTx(state, tx.id);
    }, timeout);
    // Never keep a Node event loop alive just for the idle timer.
    (tx.timer as { unref?: () => void }).unref?.();
  }

  /** Idle-timeout path: rolls back the transaction if it is still the open one. */
  private async expireTx(state: ScopeState, txId: string): Promise<void> {
    await this.locked(state, async () => {
      if (state.tx?.id !== txId) return;
      if (this.sqlite3.get_autocommit(state.db) === 0) {
        await this.sqlite3.exec(state.db, "ROLLBACK");
      }
      this.closeTx(state);
    }).catch(() => {
      // The scope may be closing; there is nothing left to roll back.
    });
  }

  private closeTx(state: ScopeState): void {
    const tx = state.tx;
    if (!tx) return;
    if (tx.timer) clearTimeout(tx.timer);
    state.tx = null;
    tx.close();
  }

  /** After a successful call: manual transaction control inside SQL text is
   * a protocol violation with a deterministic outcome, never a silent state. */
  private enforceTxDiscipline(state: ScopeState, txId: string | undefined): void {
    const autocommit = this.sqlite3.get_autocommit(state.db) !== 0;
    if (txId === undefined && state.tx === null && !autocommit) {
      // The app issued BEGIN via exec. Roll it back; transactions exist only
      // through the API — a dangling manual one would silently stop persists.
      void this.sqlite3.exec(state.db, "ROLLBACK").catch(() => {});
      throw new BrokerError(
        "INVALID_ARGUMENT",
        "manual BEGIN is not supported; use poid.db.sql.transaction()",
      );
    }
    if (txId !== undefined && autocommit) {
      // The app ended the API transaction with manual COMMIT/ROLLBACK text.
      this.closeTx(state);
      throw new BrokerError(
        "INVALID_ARGUMENT",
        "the transaction was ended inside exec; end transactions via the API",
      );
    }
  }

  /** If SQLite auto-rolled-back after an error, drop our transaction state. */
  private reconcileTx(state: ScopeState): void {
    if (state.tx && this.sqlite3.get_autocommit(state.db) !== 0) {
      this.closeTx(state);
    }
  }

  // ---------------------------------------------------------- execution

  private async run(db: DbHandle, sql: string, params: unknown[] | undefined): Promise<SqlResult> {
    if (params !== undefined) return this.runSingle(db, sql, params);
    let rows: Record<string, unknown>[] = [];
    for await (const stmt of this.sqlite3.statements(db, sql)) {
      const statementRows = await this.readRows(stmt);
      // The last row-producing statement's rows are the call's rows.
      if (statementRows !== null) rows = statementRows;
    }
    return { rows, rowsAffected: 0 };
  }

  /** The params path: the text must hold exactly one statement, verified
   * **before** anything executes — a multi-statement batch with params must
   * not run its first statement and then fail. */
  private async runSingle(db: DbHandle, sql: string, params: unknown[]): Promise<SqlResult> {
    const str = this.sqlite3.str_new(db, sql);
    let stmt: StmtHandle | null = null;
    try {
      const first = await this.sqlite3.prepare_v2(db, this.sqlite3.str_value(str));
      if (!first) return { rows: [], rowsAffected: 0 };
      stmt = first.stmt;
      const second = await this.sqlite3.prepare_v2(db, first.sql);
      if (second) {
        await this.sqlite3.finalize(second.stmt);
        throw new BrokerError(
          "INVALID_ARGUMENT",
          "params require a single statement; run multi-statement SQL without params",
        );
      }
      this.bindAll(stmt, params);
      const rows = await this.readRows(stmt);
      return { rows: rows ?? [], rowsAffected: 0 };
    } finally {
      if (stmt !== null) await this.sqlite3.finalize(stmt).catch(() => {});
      this.sqlite3.str_finish(str);
    }
  }

  /** Steps a statement to completion; returns its rows, or null when the
   * statement produces no columns (DML/DDL). Enforces the result caps. */
  private async readRows(stmt: StmtHandle): Promise<Record<string, unknown>[] | null> {
    const maxRows = this.options.maxResultRows ?? 10_000;
    const maxBytes = this.options.maxResultBytes ?? 5 * 1024 * 1024;
    const columnCount = this.sqlite3.column_count(stmt);
    const rows: Record<string, unknown>[] = [];
    let approxBytes = 0;
    for (;;) {
      const status = await this.sqlite3.step(stmt);
      if (status !== SQLITE_ROW) break;
      if (columnCount === 0) continue;
      const row: Record<string, unknown> = {};
      for (let i = 0; i < columnCount; i++) {
        const value = this.readColumn(stmt, i);
        approxBytes += approxSize(value);
        row[this.sqlite3.column_name(stmt, i)] = value;
      }
      rows.push(row);
      if (rows.length > maxRows) {
        throw new BrokerError(
          "QUOTA_EXCEEDED",
          `result exceeds ${maxRows} rows; page with LIMIT/OFFSET`,
        );
      }
      if (approxBytes > maxBytes) {
        throw new BrokerError("QUOTA_EXCEEDED", "result too large; select fewer columns or rows");
      }
    }
    return columnCount > 0 ? rows : null;
  }

  private bindAll(stmt: StmtHandle, params: unknown[]): void {
    const expected = this.sqlite3.bind_parameter_count(stmt);
    if (params.length !== expected) {
      throw new BrokerError(
        "INVALID_ARGUMENT",
        `statement expects ${expected} parameter(s), got ${params.length}`,
      );
    }
    for (let i = 0; i < params.length; i++) {
      this.bindOne(stmt, i + 1, params[i]);
    }
  }

  private bindOne(stmt: StmtHandle, index: number, value: unknown): void {
    if (value === null || value === undefined) {
      this.sqlite3.bind_null(stmt, index);
    } else if (typeof value === "boolean") {
      this.sqlite3.bind_int(stmt, index, value ? 1 : 0);
    } else if (typeof value === "number") {
      if (Number.isSafeInteger(value)) this.sqlite3.bind_int64(stmt, index, BigInt(value));
      else this.sqlite3.bind_double(stmt, index, value);
    } else if (typeof value === "bigint") {
      if (value > 9223372036854775807n || value < -9223372036854775808n) {
        throw new BrokerError("INVALID_ARGUMENT", `parameter ${index} exceeds the 64-bit range`);
      }
      this.sqlite3.bind_int64(stmt, index, value);
    } else if (typeof value === "string") {
      this.sqlite3.bind_text(stmt, index, value);
    } else if (value instanceof Uint8Array) {
      this.sqlite3.bind_blob(stmt, index, value);
    } else {
      throw new BrokerError(
        "INVALID_ARGUMENT",
        `parameter ${index} must be null, boolean, number, bigint, string, or Uint8Array`,
      );
    }
  }

  private readColumn(stmt: StmtHandle, i: number): unknown {
    const value = this.sqlite3.column(stmt, i);
    if (typeof value === "bigint") {
      // No silent precision loss (RUNTIME-API §2.1: values are JSON-shaped).
      throw new BrokerError(
        "INVALID_ARGUMENT",
        `column \`${this.sqlite3.column_name(stmt, i)}\` holds a 64-bit integer outside the ` +
          "JavaScript safe range; CAST it to TEXT in SQL",
      );
    }
    // Blob reads are views into the WASM heap; hand out a copy, never a live
    // reference (same boundary rule as the kv engines).
    if (value instanceof Uint8Array) return new Uint8Array(value);
    return value;
  }

  private async scalar(db: DbHandle, sql: string): Promise<unknown> {
    for await (const stmt of this.sqlite3.statements(db, sql)) {
      const status = await this.sqlite3.step(stmt);
      if (status === SQLITE_ROW && this.sqlite3.column_count(stmt) > 0) {
        return this.sqlite3.column(stmt, 0);
      }
    }
    return undefined;
  }

  private async totalChanges(db: DbHandle): Promise<number> {
    return asNumber(await this.scalar(db, "SELECT total_changes()"));
  }

  // --------------------------------------------------------- persistence

  /** Schedules a write-behind persist when the database changed and is in a
   * committed state. Coalesces: at most one pending persist per scope. */
  private schedulePersist(scope: Scope, state: ScopeState): void {
    if (!this.options.persist) return;
    if (state.persistQueued) return;
    if (state.vfs.writes === state.persistedWrites) return;
    if (this.sqlite3.get_autocommit(state.db) === 0) return; // tx end reschedules
    state.persistQueued = true;
    this.persistChain = this.persistChain
      .then(async () => {
        state.persistQueued = false;
        const bytes = await this.locked(state, async () => {
          if (state.closed || state.tx) return undefined;
          if (this.sqlite3.get_autocommit(state.db) === 0) return undefined;
          state.persistedWrites = state.vfs.writes;
          return state.vfs.snapshot(MAIN_DB);
        });
        if (bytes) await this.options.persist?.(scope, bytes);
      })
      .catch((err) => {
        this.persistFailure = err;
      });
  }
}

function approxSize(value: unknown): number {
  if (typeof value === "string") return 2 * value.length;
  if (value instanceof Uint8Array) return value.byteLength;
  return 8;
}

function asNumber(value: unknown): number {
  return typeof value === "number" ? value : 0;
}

function toBrokerError(err: unknown): BrokerError {
  if (err instanceof BrokerError) return err;
  if (err instanceof SQLiteError) {
    if ((err.code & 0xff) === SQLITE_FULL) {
      return new BrokerError("QUOTA_EXCEEDED", "storage quota exhausted");
    }
    // SQLite messages describe the app's own SQL; there is no filesystem
    // behind the VFS, so no host path can appear here (RUNTIME-API §9).
    return new BrokerError("INVALID_ARGUMENT", err.message);
  }
  return new BrokerError("INTERNAL", "internal reader error");
}

// ------------------------------------------------------------------ broker

/** The capability each SQL method requires — mirrored from the SDK protocol
 * for the handler's own defense in depth. */
const SQL_METHODS = new Set<Method>([
  "db.sql.exec",
  "db.sql.begin",
  "db.sql.commit",
  "db.sql.rollback",
]);
const SQL_CAPABILITY: Capability = "db.sql";

/**
 * Adapts a {@link WaSqliteEngine} to the broker's injectable `sql` handler.
 * Scope is derived from the session (the window) — never from the message
 * (SECURITY rule 3); `txId` is not a scope: it is validated against the
 * scope's own open transaction and cannot address anything else.
 */
export function sqlBrokerHandler(
  engine: WaSqliteEngine,
): (session: ReaderSession, method: Method, params: Record<string, unknown>) => Promise<unknown> {
  return async (session, method, params) => {
    if (!SQL_METHODS.has(method) || !session.capabilities.has(SQL_CAPABILITY)) {
      throw new BrokerError("PERMISSION_DENIED", `capability \`${SQL_CAPABILITY}\` not granted`);
    }
    const scope: Scope = { instanceId: session.instanceId, slot: session.currentSlot };
    const call: SqlCallOptions = { quotaBytes: session.quotaBytes };
    switch (method) {
      case "db.sql.exec": {
        const sql = expectString(params.sql, "sql");
        call.txId = optionalString(params.txId, "txId");
        return engine.exec(scope, sql, expectParams(params.params), call);
      }
      case "db.sql.begin":
        return engine.begin(scope, call);
      case "db.sql.commit":
        await engine.commit(scope, expectString(params.txId, "txId"));
        return undefined;
      case "db.sql.rollback":
        await engine.rollback(scope, expectString(params.txId, "txId"));
        return undefined;
      default:
        throw new BrokerError("NOT_AVAILABLE", `unexpected sql method \`${method}\``);
    }
  };
}

function expectString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new BrokerError("INVALID_ARGUMENT", `\`${field}\` must be a string`);
  }
  return value;
}

function optionalString(value: unknown, field: string): string | undefined {
  if (value === undefined) return undefined;
  return expectString(value, field);
}

function expectParams(value: unknown): unknown[] | undefined {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) {
    throw new BrokerError("INVALID_ARGUMENT", "`params` must be an array");
  }
  return value;
}
