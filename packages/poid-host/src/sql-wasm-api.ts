/**
 * The typed slice of the wa-sqlite API that the SQL engine uses, plus the
 * loader that instantiates it. wa-sqlite's own declarations cover only its
 * root module, so — exactly like `@poid/web`'s hand-typed WASM surface — the
 * contract is written down here once and the imported object is narrowed to
 * it. Nothing from wa-sqlite appears in `@poid/host`'s public declarations.
 */

import { Factory } from "wa-sqlite/src/sqlite-api.js";

/** SQLite fundamental datatype codes (C ABI, stable). */
export const SQLITE_INTEGER = 1;
export const SQLITE_FLOAT = 2;
export const SQLITE_TEXT = 3;
export const SQLITE_BLOB = 4;
export const SQLITE_NULL = 5;

/** `sqlite3_step` results. */
export const SQLITE_ROW = 100;
export const SQLITE_DONE = 101;

/** Result code raised when the database hits `max_page_count` (the quota). */
export const SQLITE_FULL = 13;

/** `sqlite3_open_v2` flags. */
export const SQLITE_OPEN_READWRITE = 0x02;
export const SQLITE_OPEN_CREATE = 0x04;

/** Text encoding for `create_function`. */
export const SQLITE_UTF8 = 1;

/** An opaque database connection handle. */
export type DbHandle = number;
/** An opaque prepared-statement handle. */
export type StmtHandle = number;

/** The wa-sqlite calls the engine relies on. */
export interface SqliteApi {
  libversion(): string;
  open_v2(name: string, flags?: number, vfs?: string): Promise<DbHandle>;
  close(db: DbHandle): Promise<number>;
  exec(db: DbHandle, sql: string): Promise<number>;
  statements(db: DbHandle, sql: string): AsyncIterable<StmtHandle>;
  bind_parameter_count(stmt: StmtHandle): number;
  bind_null(stmt: StmtHandle, i: number): number;
  bind_int(stmt: StmtHandle, i: number, value: number): number;
  bind_int64(stmt: StmtHandle, i: number, value: bigint): number;
  bind_double(stmt: StmtHandle, i: number, value: number): number;
  bind_text(stmt: StmtHandle, i: number, value: string): number;
  bind_blob(stmt: StmtHandle, i: number, value: Uint8Array): number;
  step(stmt: StmtHandle): Promise<number>;
  finalize(stmt: StmtHandle): Promise<number>;
  str_new(db: DbHandle, s?: string): number;
  str_value(str: number): number;
  str_finish(str: number): void;
  /** Prepares the next statement at `sqlPtr`; null when none remain. */
  prepare_v2(db: DbHandle, sqlPtr: number): Promise<{ stmt: StmtHandle; sql: number } | null>;
  column_count(stmt: StmtHandle): number;
  column_name(stmt: StmtHandle, i: number): string;
  column_type(stmt: StmtHandle, i: number): number;
  /** Typed read: number | bigint (unsafe 64-bit) | string | Uint8Array (heap
   * view — copy before it is invalidated) | null. */
  column(stmt: StmtHandle, i: number): unknown;
  changes(db: DbHandle): number;
  get_autocommit(db: DbHandle): number;
  vfs_register(vfs: object, makeDefault?: boolean): number;
  create_function(
    db: DbHandle,
    name: string,
    nArg: number,
    eTextRep: number,
    pApp: number,
    xFunc?: (context: number, values: ArrayLike<number>) => void,
    xStep?: (context: number, values: ArrayLike<number>) => void,
    xFinal?: (context: number) => void,
  ): number;
  /** Reads a UDF argument value (same typing as {@link SqliteApi.column}). */
  value(pValue: number): unknown;
  /** Sets a UDF result. */
  result(context: number, value: number | string | Uint8Array | bigint | null): void;
}

/** How to obtain the WASM binary: exact bytes (already integrity-checked by
 * the caller) or a URL the Emscripten loader may fetch from. */
export interface SqliteWasmSource {
  wasmBinary?: ArrayBuffer;
  wasmUrl?: string;
}

/**
 * Loads and instantiates wa-sqlite. The Emscripten factory is imported
 * dynamically so bundlers put the ~40 KB module glue (and let the ~550 KB
 * .wasm be located lazily) outside the reader's startup path — the engine
 * loads only when a POID actually touches `poid.db.sql` / `poid.db.docs`.
 */
export async function loadSqlite(source: SqliteWasmSource): Promise<SqliteApi> {
  const { default: factory } = await import("wa-sqlite/dist/wa-sqlite.mjs");
  const module = await factory({
    wasmBinary: source.wasmBinary,
    locateFile: source.wasmUrl ? () => source.wasmUrl as string : undefined,
  });
  return Factory(module) as SqliteApi;
}
