/**
 * Seeding an `embedded`-mode engine from a container's `data/store.json`
 * (SPEC §6.2). Shared by the Web Reader and the desktop Reader so the
 * first-open experience of an embedded-storage POID is identical everywhere.
 */

import type { IndexedDbEngine } from "./idb-engine.js";
import type { Scope } from "./engine.js";

/**
 * Seeds the engine from embedded `data/store.json` bytes, if they parse to
 * an object. Returns true when anything was seeded — the caller uses this to
 * roll the seed back if the user declines consent (a declined app must leave
 * no trace).
 */
export function seedFromStore(
  db: IndexedDbEngine,
  scope: Scope,
  data: Uint8Array | undefined,
): boolean {
  if (!data) return false;
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(data));
  } catch {
    return false;
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return false;
  db.seed(scope, parsed as Record<string, unknown>);
  return !db.isEmpty(scope);
}
