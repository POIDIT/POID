/**
 * The Web Reader's vault-mode Data Engine: the same Automerge CRDT as the
 * desktop vault (`crates/poid-vault`), compiled to WASM, with the document's
 * bytes persisted in IndexedDB (SPEC §6.5 — one engine, all platforms).
 *
 * The `DataEngine` interface is synchronous and the WASM document answers
 * synchronously from memory; persistence is a write-behind of the serialized
 * document, serialized through a single promise chain so saves never
 * interleave. `flush()` awaits the chain — the download-updated-file path
 * flushes before packing, exactly like the embedded engine.
 */

import type { DataEngine, Scope } from "@poid/host";
import type { PoidWasm, VaultDocHandle } from "./wasm-api.js";

const DEFAULT_DB = "poid-web-vault-crdt";
const STORE = "docs";

function openIdb(name: string): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(name, 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("IndexedDB open failed"));
  });
}

function idbGet(db: IDBDatabase, key: string): Promise<Uint8Array | undefined> {
  return new Promise((resolve, reject) => {
    const req = db.transaction(STORE, "readonly").objectStore(STORE).get(key);
    req.onsuccess = () => resolve(req.result as Uint8Array | undefined);
    req.onerror = () => reject(req.error ?? new Error("IndexedDB read failed"));
  });
}

function idbPut(db: IDBDatabase, key: string, value: Uint8Array): Promise<void> {
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error("IndexedDB write failed"));
  });
}

/** A vault-mode engine bound to one instance's CRDT document. */
export class VaultWasmEngine implements DataEngine {
  private readonly db: IDBDatabase;
  private readonly doc: VaultDocHandle;
  private readonly instanceId: string;
  private writes: Promise<void> = Promise.resolve();

  private constructor(db: IDBDatabase, doc: VaultDocHandle, instanceId: string) {
    this.db = db;
    this.doc = doc;
    this.instanceId = instanceId;
  }

  /** Opens (or starts) the document for `instanceId`. */
  static async open(
    wasm: PoidWasm,
    instanceId: string,
    dbName = DEFAULT_DB,
  ): Promise<VaultWasmEngine> {
    const db = await openIdb(dbName);
    const bytes = await idbGet(db, instanceId);
    const doc = bytes ? wasm.VaultDoc.load(bytes) : new wasm.VaultDoc();
    return new VaultWasmEngine(db, doc, instanceId);
  }

  /** The boundary hands scopes derived from the session; this engine serves
   * exactly one instance. A mismatch is a host bug, never app-reachable. */
  private slotOf(scope: Scope): string {
    if (scope.instanceId !== this.instanceId) {
      throw new Error("vault engine received a foreign scope");
    }
    return scope.slot;
  }

  private persist(): void {
    const bytes = this.doc.save();
    this.writes = this.writes.then(() => idbPut(this.db, this.instanceId, bytes));
  }

  /** Awaits every scheduled persist. */
  flush(): Promise<void> {
    return this.writes;
  }

  kvGet(scope: Scope, key: string): unknown | null {
    const raw = this.doc.kvGet(this.slotOf(scope), key);
    return raw === undefined ? null : (JSON.parse(raw) as unknown);
  }

  kvSet(scope: Scope, key: string, value: unknown): void {
    this.doc.kvSet(this.slotOf(scope), key, JSON.stringify(value));
    this.persist();
  }

  kvDelete(scope: Scope, key: string): void {
    this.doc.kvDelete(this.slotOf(scope), key);
    this.persist();
  }

  kvList(scope: Scope, prefix?: string): string[] {
    return this.doc.kvList(this.slotOf(scope), prefix);
  }

  kvClear(scope: Scope): void {
    this.doc.kvClear(this.slotOf(scope));
    this.persist();
  }

  usage(scope: Scope): number {
    return this.doc.usage(this.slotOf(scope));
  }
}
