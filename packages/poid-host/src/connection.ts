/**
 * The connection seam for the Data Engine (SPEC §8, RUNTIME-API §2.2/§2.3).
 *
 * When `storage.mode = "connection"`, `poid.db.sql` / `poid.db.docs` are
 * served by a **Connection** instead of the per-instance local store. The
 * application calls the identical API and never learns which backend
 * answered — and, for a real backend, never sees the credential (SPEC §7.1).
 * The broker still derives scope from the session; the only thing that
 * changes is *which* store that scope names.
 *
 * This is the seam, plus the **loopback connection** that exercises it: a
 * local SQL store keyed by the connection id, so every POID configured
 * against one connection shares a single database (the defining property of
 * a connection — data lives with the backend, not the file). Real backends
 * (Supabase, a hosted Postgres, …) arrive with the Connections milestone;
 * they slot in behind this same `makeSqlHandlers` shape without the
 * application changing at all.
 */

import { makeSqlHandlers, type SqlHandlers, type SqlPersistence } from "./sql-persistence.js";
import type { SqliteWasmSource } from "./sql-wasm-api.js";

/** Options for {@link loopbackConnection}. */
export interface LoopbackConnectionOptions {
  /** Stable id of the configured connection; the shared store's key. */
  connectionId: string;
  wasm: SqliteWasmSource;
  /** Where the loopback backend keeps its data. Local, keyed by connection
   * id — a stand-in for the external backend a real connection would use. */
  persistence?: SqlPersistence;
}

/** The scope-key prefix that marks connection-owned storage — distinct from
 * any instance id (a UUID), so a connection store can never collide with an
 * instance store even inside one persistence backend. */
const CONNECTION_SCOPE_PREFIX = "connection:";

/**
 * Builds the `sql` + `docs` handlers for a **connection-mode** POID, backed
 * by a loopback (local) SQL store shared by connection id. The returned
 * {@link SqlHandlers} plug into `mountReader`'s `handlers` exactly like the
 * instance-scoped ones — the reader picks which to pass based on
 * `storage.mode`, and the application cannot tell the difference.
 */
export function loopbackConnection(options: LoopbackConnectionOptions): SqlHandlers {
  const key = `${CONNECTION_SCOPE_PREFIX}${options.connectionId}`;
  return makeSqlHandlers({
    wasm: options.wasm,
    persistence: options.persistence,
    // Every session that reaches this handler is remapped onto the one
    // connection scope — never the window's instance. Slots do not apply to
    // a connection: its data is shared, not per-copy.
    scopeFor: () => ({ instanceId: key, slot: "" }),
  });
}

/** The scope a loopback connection stores under (export / inspection). */
export function connectionScope(connectionId: string): { instanceId: string; slot: string } {
  return { instanceId: `${CONNECTION_SCOPE_PREFIX}${connectionId}`, slot: "" };
}
