/**
 * The desktop Reader's vault-mode Data Engine: a synchronous in-memory
 * mirror of the window's active slot, hydrated once from the Rust vault and
 * written through over IPC.
 *
 * The authoritative store is `crates/poid-vault` behind the `vault_*`
 * commands — scope there is derived from the calling window's label, so this
 * engine carries no instance identifier at all. Writes are queued on one
 * promise chain (ordering) and `flush()` awaits it; the broker's quota check
 * runs against `usage()` *before* a write, mirroring the Rust-side
 * projection, so the two sides agree on when `QUOTA_EXCEEDED` fires.
 */

import type { DataEngine, Scope } from "@poid/host";
import { invoke } from "@tauri-apps/api/core";

interface VaultEntryDto {
  key: string;
  valueJson: string;
}

interface VaultSnapshotDto {
  slot: string;
  slots: string[];
  entries: VaultEntryDto[];
}

function jsonClone(value: unknown): unknown {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

/** A vault engine bound to this window's active slot. */
export class VaultIpcEngine implements DataEngine {
  private readonly entries = new Map<string, unknown>();
  /** The active slot at hydration; the scope the broker builds must match. */
  readonly slot: string;
  /** Every slot with stored data (for the reader's slot switcher). */
  readonly slots: string[];
  private writes: Promise<void> = Promise.resolve();
  private failed: unknown = null;

  private constructor(snapshot: VaultSnapshotDto) {
    this.slot = snapshot.slot;
    this.slots = snapshot.slots;
    for (const entry of snapshot.entries) {
      this.entries.set(entry.key, JSON.parse(entry.valueJson) as unknown);
    }
  }

  /** Hydrates the engine for the calling window. */
  static async open(): Promise<VaultIpcEngine> {
    const snapshot = await invoke<VaultSnapshotDto>("vault_hydrate");
    return new VaultIpcEngine(snapshot);
  }

  private guard(scope: Scope): void {
    if (scope.slot !== this.slot) {
      // The reader remounts on slot switch; a mismatched scope is a host bug.
      throw new Error("vault engine scope does not match the hydrated slot");
    }
  }

  private queue(op: () => Promise<void>): void {
    this.writes = this.writes.then(op).catch((e) => {
      // Surfaced on flush(); the in-memory mirror stays consistent with what
      // the app observed, and the next flush reports the persistence failure.
      this.failed = e;
    });
  }

  /** Awaits every queued write; rejects if any failed. */
  async flush(): Promise<void> {
    await this.writes;
    if (this.failed !== null) {
      const e = this.failed;
      this.failed = null;
      throw e instanceof Error ? e : new Error(String(e));
    }
  }

  kvGet(scope: Scope, key: string): unknown | null {
    this.guard(scope);
    return this.entries.has(key) ? jsonClone(this.entries.get(key)) : null;
  }

  kvSet(scope: Scope, key: string, value: unknown): void {
    this.guard(scope);
    this.entries.set(key, jsonClone(value));
    const valueJson = JSON.stringify(value);
    this.queue(() => invoke("vault_kv_set", { key, valueJson }));
  }

  kvDelete(scope: Scope, key: string): void {
    this.guard(scope);
    this.entries.delete(key);
    this.queue(() => invoke("vault_kv_delete", { key }));
  }

  kvList(scope: Scope, prefix?: string): string[] {
    this.guard(scope);
    const keys = [...this.entries.keys()];
    return (prefix ? keys.filter((k) => k.startsWith(prefix)) : keys).sort();
  }

  kvClear(scope: Scope): void {
    this.guard(scope);
    this.entries.clear();
    this.queue(() => invoke("vault_kv_clear"));
  }

  usage(scope: Scope): number {
    this.guard(scope);
    let total = 0;
    for (const [key, value] of this.entries) {
      total += key.length + JSON.stringify(value).length;
    }
    return total;
  }
}

/** Switches the window's active slot in the vault and returns the engine's
 * fresh hydration. The caller remounts the application afterwards — the app
 * must start over in the new scope, never observe the switch (SPEC §6.4). */
export async function switchSlot(slot: string): Promise<VaultIpcEngine> {
  await invoke<VaultSnapshotDto>("vault_switch_slot", { slot });
  return VaultIpcEngine.open();
}
