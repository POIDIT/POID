#!/usr/bin/env node
/**
 * Manual verification of the `sql` connection kind against a real database
 * (M11.7). CI cannot do this: it needs a real server and a real credential,
 * and putting either in the repository would be the exact mistake this
 * milestone exists to prevent.
 *
 * What it proves, end to end:
 *   1. The DSN parses the way `poid-connections` parses it.
 *   2. The database is reachable and accepts a statement.
 *   3. A table survives a write and a read — the thing an application does.
 *   4. The credential does not appear in anything POID persists.
 *
 * Usage:
 *   node scripts/verify-sql-connection.mjs "postgres://user:pass@host:5432/db"
 *
 * The DSN is read from argv and never written anywhere. Run it from a shell
 * whose history you do not keep, or export it and pass "$POID_TEST_DSN".
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const dsn = process.argv[2];
if (!dsn) {
  console.error("usage: node scripts/verify-sql-connection.mjs <postgres-dsn>");
  console.error("");
  console.error("Get one from Supabase: Project Settings → Database →");
  console.error("Connection string → URI. Use the pooler URI on port 6543.");
  process.exit(2);
}

/** Everything the script has proved or failed to prove. */
const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "  ok  " : " FAIL "} ${name}${detail ? ` — ${detail}` : ""}`);
}

// ---------------------------------------------------------------- 1. the DSN

const parsed = /^postgres(?:ql)?:\/\/([^:@]+)(?::([^@]*))?@([^/]+)\/([^?]+)/.exec(dsn);
check("the DSN has the shape poid-connections expects", parsed !== null);
if (!parsed) process.exit(1);

const [, user, password, authority, database] = parsed;
check(
  "it names a user, a host and a database",
  Boolean(user && authority && database),
  `${user}@${authority}/${database}`,
);
check("it carries a password", Boolean(password && password.length > 0));

// ------------------------------------------------- 2 & 3. reach the database
//
// Driven through the same Rust crate the product uses, so this exercises the
// real code path rather than a second implementation that might disagree.

const probe = `
use poid_connections::PostgresConnection;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let dsn = std::env::var("POID_VERIFY_DSN").expect("POID_VERIFY_DSN");
    let client = match PostgresConnection::open(&dsn).await {
        Ok(c) => c,
        Err(e) => { println!("CONNECT_FAILED {e}"); std::process::exit(1); }
    };
    let table = "poid_verify_tmp";
    for (label, sql) in [
        ("create", format!("CREATE TABLE IF NOT EXISTS {table} (id int primary key, note text)")),
        ("clear", format!("DELETE FROM {table}")),
    ] {
        if let Err(e) = client.execute(&sql, &[]).await {
            println!("{label}_FAILED {e}");
            std::process::exit(1);
        }
    }
    if let Err(e) = client
        .execute(&format!("INSERT INTO {table}(id, note) VALUES ($1, $2)"),
                 &[serde_json::json!(1), serde_json::json!("hello from POID")]).await {
        println!("INSERT_FAILED {e}");
        std::process::exit(1);
    }
    match client.exec(&format!("SELECT id, note FROM {table} ORDER BY id"), &[]).await {
        Ok(r) => println!("ROWS {}", serde_json::to_string(&r.rows).unwrap_or_default()),
        Err(e) => { println!("SELECT_FAILED {e}"); std::process::exit(1); }
    }
    let _ = client.execute(&format!("DROP TABLE {table}"), &[]).await;
    println!("DONE");
}
`;

console.log("\nBuilding the probe against the product's own crate…");
const probeDir = join(process.cwd(), "target", "verify-sql");
const mk = spawnSync("cargo", ["--version"], { encoding: "utf8" });
if (mk.status !== 0) {
  console.error("cargo is not available; this script needs the Rust toolchain.");
  process.exit(2);
}

// A tiny throwaway crate that depends on the real one.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync(join(probeDir, "src"), { recursive: true });
writeFileSync(
  join(probeDir, "Cargo.toml"),
  `[package]
name = "poid-verify-sql"
version = "0.0.0"
edition = "2021"

[dependencies]
poid-connections = { path = "${join(process.cwd(), "crates", "poid-connections").replace(/\\/g, "/")}" }
tokio = { version = "1", features = ["rt", "macros"] }
serde_json = "1"

[workspace]
`,
);
writeFileSync(join(probeDir, "src", "main.rs"), probe);

const run = spawnSync(
  "cargo",
  ["run", "--quiet", "--manifest-path", join(probeDir, "Cargo.toml")],
  {
    encoding: "utf8",
    env: { ...process.env, POID_VERIFY_DSN: dsn },
  },
);

const output = `${run.stdout ?? ""}${run.stderr ?? ""}`;
check(
  "the database accepts a connection",
  !output.includes("CONNECT_FAILED"),
  output.includes("CONNECT_FAILED") ? output.split("CONNECT_FAILED")[1]?.trim().slice(0, 200) : "",
);
check("a table can be created and written", !/(create|clear|INSERT)_FAILED/.test(output));

const rows = /ROWS (.*)/.exec(output);
check(
  "the row comes back through poid.db.sql's own path",
  rows !== null,
  rows ? rows[1] : output.slice(0, 200),
);
if (rows) {
  let parsedRows;
  try {
    parsedRows = JSON.parse(rows[1]);
  } catch {
    parsedRows = [];
  }
  check(
    "the value round-trips exactly",
    parsedRows.length === 1 && parsedRows[0]?.note === "hello from POID",
  );
}

// ------------------------------------------- 4. the credential is not on disk

const appData =
  process.platform === "win32"
    ? join(process.env.APPDATA ?? homedir(), "dev.poid.studio")
    : process.platform === "darwin"
      ? join(homedir(), "Library", "Application Support", "dev.poid.studio")
      : join(homedir(), ".local", "share", "dev.poid.studio");

let searched = 0;
const leaked = [];
for (const name of ["connections.json", "bindings.json"]) {
  const path = join(appData, name);
  if (!existsSync(path)) continue;
  searched += 1;
  const text = readFileSync(path, "utf8");
  if (password && text.includes(password)) leaked.push(path);
  if (text.includes(dsn)) leaked.push(path);
}
check(
  searched > 0
    ? "no POID file on disk contains the credential"
    : "no POID files to search yet (configure the connection in Studio first)",
  leaked.length === 0,
  leaked.join(", "),
);

// --------------------------------------------------------------------- verdict

const failed = results.filter((r) => !r.ok);
console.log("");
if (failed.length === 0) {
  console.log(`All ${results.length} checks passed.`);
  console.log("`poid.db.sql` works against this database, through the product's own code.");
  process.exit(0);
}
console.log(`${failed.length} of ${results.length} checks failed.`);
process.exit(1);
