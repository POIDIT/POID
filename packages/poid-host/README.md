# @poid/host

The host side of the sandbox bridge and the **boundary broker** — the security
boundary between a POID application and the reader (SECURITY.md, SPEC §5.2, §7).

- Creates the sandboxed iframe (`sandbox="allow-scripts"`, never
  `allow-same-origin`), serves the container from an opaque origin, and applies
  the CSP (`connect-src 'none'` by default).
- The broker brokers every `window.poid` call with four executable invariants:
  scope is derived from the sending window (never the message); unknown method
  or field fails closed; no credential ever crosses the boundary; every call is
  rate-limited and quota-checked.
- Watchdog: an app that hangs its event loop stops answering pings and is
  marked unresponsive; the host-side "Stop this application" always works.

**Any change here requires a security review note in the PR (CONTRIBUTING.md).**

## The Data Engine (M10)

`poid.db.kv` / `sql` / `docs` are brokered here too, scope derived from the
window like everything else:

- **SQL** (`sql-engine.ts`): SQLite compiled to WASM (wa-sqlite), one database
  per `(instanceId, slot)` scope inside a private in-memory VFS (`sql-vfs.ts`).
  Isolation is structural — even `ATTACH DATABASE` cannot reach another scope.
  Quota is `PRAGMA max_page_count` from `storage.quota_mb`; transactions hold a
  logical lock with an idle-timeout auto-rollback.
- **Documents** (`docs-store.ts` + `docs-query.ts`): one table per collection,
  the Mongo-like query subset compiled to SQLite JSON functions with every
  value bound as a parameter. Shallow-merge `update`, `$regex` via a registered
  UDF.
- **Wiring** (`sql-persistence.ts`): the ~550 KB engine loads lazily on the
  first `db.sql`/`db.docs` call; `SqlPersistence` is the platform port
  (IndexedDB on web, the Rust vault over IPC on desktop). `data/database.sql`
  is the human-readable embedded form (`sql-dump.ts`).
- **Connections** (`connection.ts`): `storage.mode = "connection"` routes to a
  loopback connection keyed by app id — the same API, a shared backend.
- **Migrations** (`migrations.ts`): the SPEC §12 "update program, keep data"
  flow, tracked with `PRAGMA user_version`.

The engine runs the real wa-sqlite WASM in Node under vitest; the Playwright
`sql.spec.ts` proves it end to end through the sandbox (docs CRUD, restart
survival, the 10k-row < 50 ms perf bound).

```
pnpm --filter @poid/host test    # Node logic tier (broker, guard, bridge, watchdog)
pnpm --filter @poid/host e2e      # Playwright tier: real-browser enforcement
```

The Playwright tier proves the browser-enforced boundaries (CSP blocks fetch,
iframe escape, opaque origin, watchdog kill). It needs a Chromium:
`pnpm exec playwright install chromium`.
