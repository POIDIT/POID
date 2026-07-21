/**
 * SQL text dump — the canonical embedded form of the SQL tier (M10, SPEC
 * §6.2 spirit): `data/database.sql` is human-readable UTF-8 that any SQLite
 * can replay, so a user can always recover their data with a ZIP tool and
 * sqlite3. The reader executes it into a fresh database on first open and
 * regenerates it on export.
 *
 * Determinism: schema objects sort by (kind, name); table rows dump in rowid
 * order (or PK order for WITHOUT ROWID tables). Every literal is encoded by
 * SQLite's own `quote()` — floats round-trip, blobs become X'…', and 64-bit
 * integers never touch a JavaScript number.
 */

import type { Scope } from "./engine.js";
import type { WaSqliteEngine } from "./sql-engine.js";

/** Doubles quotes in an identifier taken from sqlite_master. */
function ident(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

interface MasterRow {
  type: string;
  name: string;
  sql: string;
}

/**
 * Serialises a scope's database to SQL text, or null when it holds no user
 * objects. The text replays as a single `exec` batch: foreign keys off, one
 * transaction, foreign keys back on.
 */
export async function dumpSql(engine: WaSqliteEngine, scope: Scope): Promise<string | null> {
  const master = await engine.exec(
    scope,
    "SELECT type, name, sql FROM sqlite_master " +
      "WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' " +
      "ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'trigger' THEN 2 ELSE 3 END, name",
  );
  const objects = master.rows as unknown as MasterRow[];
  if (objects.length === 0) return null;

  const lines: string[] = ["PRAGMA foreign_keys=OFF;", "BEGIN;"];

  for (const object of objects.filter((o) => o.type === "table")) {
    lines.push(`${object.sql.trimEnd().replace(/;$/, "")};`);
    lines.push(...(await dumpRows(engine, scope, object.name)));
  }

  // AUTOINCREMENT counters survive the round-trip (SQLite's own .dump does
  // the same). `sqlite_sequence` exists iff some CREATE above used
  // AUTOINCREMENT, and replaying the data INSERTs already recreated its rows;
  // reset them to the recorded high-water marks. The table has no UNIQUE
  // constraint, so it is DELETE + plain INSERT, not upsert.
  const sequences = await tryRows(
    engine,
    scope,
    "SELECT quote(name) AS n, seq FROM sqlite_sequence",
  );
  if (sequences.length > 0) {
    lines.push("DELETE FROM sqlite_sequence;");
    for (const row of sequences) {
      lines.push(
        `INSERT INTO sqlite_sequence(name, seq) VALUES (${row.n}, ${Number(row.seq) || 0});`,
      );
    }
  }

  for (const object of objects.filter((o) => o.type !== "table")) {
    lines.push(`${object.sql.trimEnd().replace(/;$/, "")};`);
  }

  lines.push("COMMIT;", "PRAGMA foreign_keys=ON;");
  return `${lines.join("\n")}\n`;
}

/** Rows per page while dumping — stays under the engine's result cap. */
const DUMP_PAGE = 4000;

async function dumpRows(engine: WaSqliteEngine, scope: Scope, table: string): Promise<string[]> {
  const info = await engine.exec(scope, `PRAGMA table_info(${ident(table)})`);
  const columns = info.rows.map((r) => String(r.name));
  if (columns.length === 0) return [];
  const selectList = columns.map((c) => `quote(${ident(c)})`).join(", ");
  const columnList = columns.map(ident).join(", ");

  // rowid order for determinism; WITHOUT ROWID tables have no rowid and
  // already iterate in primary-key order. Paged, so a large table never
  // trips the engine's per-call result cap.
  const lines: string[] = [];
  for (let offset = 0; ; offset += DUMP_PAGE) {
    let rows: Record<string, unknown>[];
    const page = `LIMIT ${DUMP_PAGE} OFFSET ${offset}`;
    try {
      rows = (
        await engine.exec(scope, `SELECT ${selectList} FROM ${ident(table)} ORDER BY rowid ${page}`)
      ).rows;
    } catch {
      rows = (await engine.exec(scope, `SELECT ${selectList} FROM ${ident(table)} ${page}`)).rows;
    }
    for (const row of rows) {
      const literals = Object.values(row).map(String).join(", ");
      lines.push(`INSERT INTO ${ident(table)} (${columnList}) VALUES (${literals});`);
    }
    if (rows.length < DUMP_PAGE) break;
  }
  return lines;
}

async function tryRows(
  engine: WaSqliteEngine,
  scope: Scope,
  sql: string,
): Promise<Record<string, unknown>[]> {
  try {
    return (await engine.exec(scope, sql)).rows;
  } catch {
    return [];
  }
}
