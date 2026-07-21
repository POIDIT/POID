/**
 * Schema migrations for the SQL tier (SPEC §12): the "Update program, keep
 * data" flow. When an application is updated in place — `app/` swapped, but
 * `data/` and `instance.id` preserved — its data database may sit at an
 * older schema than the new code expects. The reader bridges the gap by
 * applying the container's `migrations/NNNN-name.sql` in order.
 *
 * Model (deliberately the simple, safe one):
 * - `PRAGMA user_version` on each scope's database records the schema
 *   version its data is at.
 * - A **fresh** database is stamped at the manifest's current
 *   `storage.schema_version`: a new install builds the current schema from
 *   the app's own code (the docs store's `CREATE TABLE IF NOT EXISTS`, or an
 *   embedded `data/database.sql`), so migrations never run against an empty
 *   database.
 * - An **existing** database whose recorded version is below the manifest's
 *   version gets every migration between the two, in ascending order, each
 *   in its own transaction that also advances `user_version` — so a crash
 *   mid-update re-runs only the migration that did not commit, never one
 *   that did.
 *
 * Migrations are SQL only. The kv tier is schemaless and migrates in app
 * code; running container code outside the sandbox to migrate data would
 * breach the security model, so it is not an option (see AGENT-CONTEXT).
 */

import { BrokerError } from "./broker.js";
import type { Scope } from "./engine.js";
import type { SqlCallOptions, WaSqliteEngine } from "./sql-engine.js";

/** One ordered migration script, parsed from `migrations/NNNN-name.sql`. */
export interface Migration {
  /** The ordinal `NNNN` — the schema version this migration produces. */
  version: number;
  /** The `name` part of the filename, for diagnostics. */
  name: string;
  /** The script body. */
  sql: string;
}

/** `migrations/0003-add-labels.sql` → {version: 3, name: "add-labels"}. */
const MIGRATION_PATH = /^migrations\/(\d{1,9})-([A-Za-z0-9._-]+)\.sql$/;

/**
 * Parses the container's `migrations/` entries into an ordered, validated
 * list. Rejects duplicate ordinals and non-conforming names (fail closed —
 * a migration set the reader cannot order unambiguously must not run).
 */
export function parseMigrations(files: Map<string, Uint8Array>): Migration[] {
  const decoder = new TextDecoder();
  const byVersion = new Map<number, Migration>();
  for (const [path, bytes] of files) {
    if (!path.startsWith("migrations/")) continue;
    const match = MIGRATION_PATH.exec(path);
    if (!match) {
      throw new BrokerError(
        "INVALID_ARGUMENT",
        `migration \`${path}\` must be named migrations/NNNN-name.sql`,
      );
    }
    const version = Number.parseInt(match[1] as string, 10);
    if (version <= 0) {
      throw new BrokerError(
        "INVALID_ARGUMENT",
        `migration \`${path}\` must have a positive number`,
      );
    }
    if (byVersion.has(version)) {
      throw new BrokerError("INVALID_ARGUMENT", `two migrations share number ${version}`);
    }
    byVersion.set(version, { version, name: match[2] as string, sql: decoder.decode(bytes) });
  }
  return [...byVersion.values()].sort((a, b) => a.version - b.version);
}

/** Reads a scope's recorded schema version (`PRAGMA user_version`). */
export async function schemaVersionOf(
  engine: WaSqliteEngine,
  scope: Scope,
  call: SqlCallOptions = {},
): Promise<number> {
  const { rows } = await engine.exec(scope, "PRAGMA user_version", undefined, call);
  const value = rows[0]?.user_version;
  return typeof value === "number" ? value : 0;
}

/** Stamps a fresh database at `version` (autocommit; persisted). */
export async function stampSchemaVersion(
  engine: WaSqliteEngine,
  scope: Scope,
  version: number,
  call: SqlCallOptions = {},
): Promise<void> {
  // user_version is a header field; the value is inlined (it is a non-negative
  // integer we control, never application input).
  await engine.exec(scope, `PRAGMA user_version = ${version | 0}`, undefined, call);
}

/**
 * Applies every migration whose version is above the database's recorded
 * version and at or below `target`, in ascending order. Each runs in its own
 * transaction that also advances `user_version`, so the pair is atomic.
 * Returns the versions that were applied.
 */
export async function applyMigrations(
  engine: WaSqliteEngine,
  scope: Scope,
  target: number,
  migrations: Migration[],
  call: SqlCallOptions = {},
): Promise<number[]> {
  const current = await schemaVersionOf(engine, scope, call);
  if (target < current) {
    // The data is newer than the code — a downgrade. Refuse rather than
    // guess: there is no down-migration path, and running forward scripts
    // would corrupt newer data.
    throw new BrokerError(
      "INVALID_ARGUMENT",
      `this data is at schema version ${current}, newer than the application's ${target}; ` +
        "open it with a matching or newer version of the application",
    );
  }
  const pending = migrations
    .filter((m) => m.version > current && m.version <= target)
    .sort((a, b) => a.version - b.version);

  const applied: number[] = [];
  for (const migration of pending) {
    const tx = await engine.begin(scope, call);
    try {
      await engine.exec(scope, migration.sql, undefined, { ...call, txId: tx });
      await engine.exec(scope, `PRAGMA user_version = ${migration.version}`, undefined, {
        ...call,
        txId: tx,
      });
      await engine.commit(scope, tx);
    } catch (err) {
      await engine.rollback(scope, tx).catch(() => {});
      const detail = err instanceof Error ? err.message : String(err);
      throw new BrokerError(
        "INTERNAL",
        `migration ${migration.version} (${migration.name}) failed and was rolled back: ${detail}`,
      );
    }
    applied.push(migration.version);
  }
  return applied;
}
