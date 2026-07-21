/**
 * The desktop Reader's vault-mode SQL persistence (M10.3): database bytes
 * travel to `crates/poid-vault` as base64 over IPC. Scope discipline matches
 * every other vault command — the Rust side resolves the instance and the
 * active slot from the calling window's label, so this module sends bytes
 * and nothing else.
 */

import type { Scope, SqlPersistence } from "@poid/host";
import { invoke } from "@tauri-apps/api/core";

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function fromBase64(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Vault-backed persistence for this window's instance + active slot. */
export class VaultSqlPersistence implements SqlPersistence {
  async load(_scope: Scope): Promise<Uint8Array | undefined> {
    // Scope comes from the window on the Rust side; the parameter is only
    // the engine's addressing within this window and is deliberately unused.
    const b64 = await invoke<string | null>("vault_sql_load");
    return b64 === null ? undefined : fromBase64(b64);
  }

  async persist(_scope: Scope, bytes: Uint8Array): Promise<void> {
    await invoke("vault_sql_save", { bytesB64: toBase64(bytes) });
  }
}
