/**
 * Ambient declarations for the wa-sqlite subpath modules this package uses.
 * The package's own `types` field only covers its root import; the precise
 * surface we rely on is typed locally in `sql-wasm-api.ts` — these
 * declarations stay deliberately loose so nothing from wa-sqlite leaks into
 * the public `@poid/host` declaration files.
 */

declare module "wa-sqlite/dist/wa-sqlite.mjs" {
  /** The Emscripten module factory for the synchronous build. */
  const factory: (config?: {
    wasmBinary?: ArrayBuffer;
    locateFile?(file: string, prefix: string): string;
  }) => Promise<unknown>;
  export default factory;
}

declare module "wa-sqlite/src/sqlite-api.js" {
  /** Builds the SQLite API object from an instantiated Emscripten module. */
  export function Factory(module: unknown): unknown;
  /** Error thrown by every wrapped SQLite call; `code` is the C result code. */
  export class SQLiteError extends Error {
    readonly code: number;
  }
}

declare module "wa-sqlite/src/VFS.js" {
  /** Result and open-flag constants (re-exported sqlite-constants). */
  export const SQLITE_OK: number;
  export const SQLITE_CANTOPEN: number;
  export const SQLITE_IOERR_SHORT_READ: number;
  export const SQLITE_OPEN_CREATE: number;
  export const SQLITE_OPEN_DELETEONCLOSE: number;

  /** Base class for a VFS: lock/sync ops default to OK, file ops to IOERR. */
  export class Base {
    mxPathName: number;
    xOpen(name: string | null, fileId: number, flags: number, pOutFlags: DataView): number;
    xClose(fileId: number): number;
    xRead(fileId: number, pData: Uint8Array, iOffset: number): number;
    xWrite(fileId: number, pData: Uint8Array, iOffset: number): number;
    xTruncate(fileId: number, iSize: number): number;
    xFileSize(fileId: number, pSize64: DataView): number;
    xDelete(name: string, syncDir: number): number;
    xAccess(name: string, flags: number, pResOut: DataView): number;
  }
}
