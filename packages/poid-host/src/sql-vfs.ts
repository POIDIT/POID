/**
 * A private in-memory VFS per (instanceId, slot) scope — the structural
 * isolation layer of the SQL engine.
 *
 * SQLite reaches every file through its VFS, addressed by name. Because each
 * scope gets its **own** VFS instance holding only that scope's database,
 * there is no name an application could pass — including via `ATTACH
 * DATABASE` — that reaches another scope's data: any unknown name simply
 * creates a fresh empty file inside the same scope. Isolation needs no SQL
 * inspection at all (SECURITY rule 3: scope is derived from the session).
 *
 * The engine also uses this class as its persistence port: `snapshot()`
 * copies the database file's current bytes, `load()` seeds them before
 * SQLite first opens the file, and `writes` is a monotonic counter the
 * engine reads as its "something changed on disk" signal.
 */

import * as VFS from "wa-sqlite/src/VFS.js";

interface MemFile {
  name: string;
  flags: number;
  size: number;
  data: ArrayBuffer;
}

/** One scope's in-memory filesystem. */
export class ScopeVfs extends VFS.Base {
  /** The VFS registration name (unique per scope, engine-assigned). */
  readonly name: string;
  /** Files by name. */
  private readonly files = new Map<string, MemFile>();
  /** Open files by SQLite file id. */
  private readonly handles = new Map<number, MemFile>();
  /** Bumped on every mutation SQLite performs — the engine's dirty signal. */
  writes = 0;

  constructor(name: string) {
    super();
    this.name = name;
  }

  /** Seeds `file` with `bytes`. Only valid before SQLite opens the file. */
  load(file: string, bytes: Uint8Array): void {
    if (this.files.has(file)) {
      throw new Error(`vfs file \`${file}\` already exists; load() must precede open`);
    }
    const data = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(data).set(bytes);
    this.files.set(file, {
      name: file,
      flags: VFS.SQLITE_OPEN_CREATE,
      size: bytes.byteLength,
      data,
    });
  }

  /** Copies the current contents of `file`; undefined when it does not exist. */
  snapshot(file: string): Uint8Array | undefined {
    const f = this.files.get(file);
    if (!f) return undefined;
    return new Uint8Array(f.data.slice(0, f.size));
  }

  /** The current size of `file` in bytes (0 when absent). */
  fileSize(file: string): number {
    return this.files.get(file)?.size ?? 0;
  }

  override xOpen(name: string | null, fileId: number, flags: number, pOutFlags: DataView): number {
    // Anonymous (temp) files get a private generated name.
    const key = name ?? `.anon-${this.writes}-${this.handles.size}`;
    let file = this.files.get(key);
    if (!file) {
      if (!(flags & VFS.SQLITE_OPEN_CREATE)) return VFS.SQLITE_CANTOPEN;
      file = { name: key, flags, size: 0, data: new ArrayBuffer(0) };
      this.files.set(key, file);
    }
    this.handles.set(fileId, file);
    pOutFlags.setInt32(0, flags, true);
    return VFS.SQLITE_OK;
  }

  override xClose(fileId: number): number {
    const file = this.handles.get(fileId);
    this.handles.delete(fileId);
    if (file && file.flags & VFS.SQLITE_OPEN_DELETEONCLOSE) {
      this.files.delete(file.name);
    }
    return VFS.SQLITE_OK;
  }

  override xRead(fileId: number, pData: Uint8Array, iOffset: number): number {
    const file = this.handles.get(fileId);
    if (!file) return VFS.SQLITE_CANTOPEN;
    const begin = Math.min(iOffset, file.size);
    const end = Math.min(iOffset + pData.byteLength, file.size);
    const count = end - begin;
    if (count > 0) pData.set(new Uint8Array(file.data, begin, count));
    if (count < pData.byteLength) {
      pData.fill(0, count);
      return VFS.SQLITE_IOERR_SHORT_READ;
    }
    return VFS.SQLITE_OK;
  }

  override xWrite(fileId: number, pData: Uint8Array, iOffset: number): number {
    const file = this.handles.get(fileId);
    if (!file) return VFS.SQLITE_CANTOPEN;
    if (iOffset + pData.byteLength > file.data.byteLength) {
      const grown = new ArrayBuffer(Math.max(iOffset + pData.byteLength, 2 * file.data.byteLength));
      new Uint8Array(grown).set(new Uint8Array(file.data, 0, file.size));
      file.data = grown;
    }
    new Uint8Array(file.data, iOffset, pData.byteLength).set(pData);
    file.size = Math.max(file.size, iOffset + pData.byteLength);
    this.writes += 1;
    return VFS.SQLITE_OK;
  }

  override xTruncate(fileId: number, iSize: number): number {
    const file = this.handles.get(fileId);
    if (!file) return VFS.SQLITE_CANTOPEN;
    file.size = Math.min(file.size, iSize);
    this.writes += 1;
    return VFS.SQLITE_OK;
  }

  override xFileSize(fileId: number, pSize64: DataView): number {
    const file = this.handles.get(fileId);
    if (!file) return VFS.SQLITE_CANTOPEN;
    pSize64.setBigInt64(0, BigInt(file.size), true);
    return VFS.SQLITE_OK;
  }

  override xDelete(name: string): number {
    if (this.files.delete(name)) this.writes += 1;
    return VFS.SQLITE_OK;
  }

  override xAccess(name: string, _flags: number, pResOut: DataView): number {
    pResOut.setInt32(0, this.files.has(name) ? 1 : 0, true);
    return VFS.SQLITE_OK;
  }
}
