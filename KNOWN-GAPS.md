# Known gaps

What is deliberately unfinished, and what is finished but unverified. Kept in
the repository rather than in someone's head, because the failure mode of a
milestone is not "we forgot to build it" — it is "we forgot that we knew".

Each entry says what is wrong, what it costs, and what would close it.

Last reviewed at the end of **M11 (Connections & the Credential Broker)**.

---

## Bugs

### The row count of a write through a Connection can be wrong

`apps/studio/src-tauri/src/connections.rs` — `connection_sql_exec` decides
whether a statement returns rows by **sniffing the SQL text**:

```rust
let returns_rows = trimmed.starts_with("select") || … || trimmed.contains(" returning ");
```

Two ways it is wrong:

- `UPDATE notes SET body = 'see returning policy'` contains `" returning "` in
  a string literal, so it is routed to the row-returning path and reports
  `rowsAffected: 0` instead of the real count.
- `INSERT … RETURNING id` returns its rows correctly but also reports
  `rowsAffected: 0`, because `PostgresConnection::exec` hard-codes zero.

Neither loses data and neither is a security issue — the rows themselves are
right. An application that trusts `rowsAffected` gets a wrong number.

**Fix:** delete the heuristic. `tokio_postgres`'s `query_raw` returns a
`RowStream` that carries both the rows and `rows_affected()`, so the driver's
own count replaces the guess and no text is parsed at all. Needs a
`futures-util` dependency for the stream.

**Why it was not fixed in M11:** it is a change to a code path that cannot be
exercised without a real Postgres server (see *Unverified* below), and an
untested fix to an untestable path is a worse state than a known bug.

### A relative redirect resolves against the origin, not the path

`crates/poid-connections/src/http.rs` — `resolve_redirect` builds
`{origin}/{location}` for a relative destination, ignoring the current path.

- `https://a.test/one` + `two` → `https://a.test/two` — correct by accident.
- `https://a.test/dir/page` + `two` → `https://a.test/two`, but RFC 3986 says
  `https://a.test/dir/two`.

Not a security hole: the origin is still re-checked on every hop, so this
cannot reach somewhere the allowlist forbids. It reaches the *wrong path* on an
allowed origin.

**Fix:** implement RFC 3986 §5.3 reference resolution, or take a URL crate.
The existing unit test encodes the root-relative case only and should grow a
nested-path case first.

### `connection_save` does not pin a connection's kind on edit

`apps/studio/src-tauri/src/connections.rs`. The hub's UI disables the kind
selector when editing, but the IPC command does not enforce it — a caller
passing an existing `id` with a different `kind` would rewrite the record's
configuration. Only Studio's own hub window can call it (`hub_only`), so this
is defence in depth rather than a live hole.

**Fix:** reject a mismatch between the supplied kind and the stored record's.

---

## Broken infrastructure

### The Cloudflare Workers deployment (`poiddev`) has never succeeded

`Workers Builds: poiddev` fails on every commit, on `main` as well as on
branches, going back to when the integration was connected around `d0ec82b`
(M10.7). It has not passed once. It is **not** a regression from any milestone
since, and a red check on a PR does not mean that PR broke anything.

What is known:

- The repository side of the pipeline is healthy. The documented build command
  in `packages/poid-web/wrangler.jsonc` —
  `pnpm --filter @poid/web build:wasm && pnpm --filter @poid/web build:site` —
  runs clean locally, including the `wasm32-unknown-unknown` build.
- The failure is **instantaneous**: the check's `started_at` and `completed_at`
  are the same second. A build that fails in zero seconds has not compiled
  anything, which points at the deployment configuration rather than at the
  code.
- A plausible cause is that Cloudflare's Workers Builds image has no Rust
  toolchain, so the `rustup target add …` that the build command starts with
  fails before anything else runs. Unconfirmed.

**Only the maintainer can diagnose this**: the logs are in the Cloudflare
dashboard, behind their account. The check's `details_url` points at the build.

**Likely fix if the cause is the missing toolchain:** the built WASM is already
committed (`packages/poid-web/src/wasm/`, `site/wasm/`), so the deployment does
not need to build it. Dropping `rustup`/`build:wasm` from the dashboard's build
command would leave `pnpm install && pnpm --filter @poid/web build:site`, which
needs only Node. This belongs in its own branch, not in a milestone PR.

---

## Unverified

### Nobody has run POID Studio with any of this

The single most important gap. CONVENTIONS' Definition of Done includes *"the
maintainer can run it and see the promised result"*, and that box is unticked
for M11.

Everything is covered by unit, integration and end-to-end tests, but the e2e
tier drives the **web** reader stack in Chromium — not the Tauri application.
These paths have never executed in the running product:

- the connection manager in the hub (add, edit, delete, the missing-credential state)
- the binding prompt in a real Reader window
- `poid.db.sql` → IPC → Postgres from an actual POID
- `poid.net` from an actual POID

**Closes it:** open Studio, add a connection, open a connection-mode POID,
answer the prompt, and watch it store a row. Then write down what broke.

### The SQL connection has never spoken to a Postgres server

The wire protocol is `tokio-postgres`'s responsibility and is widely used; what
is untested is *our* glue — the JSON↔Postgres type mapping in `column_to_json`,
the `JsonParam` binding, and the row-count path above.

There is no local Postgres and no container runtime on the development machine,
and a hand-written fake server would mostly prove that the fake matches the
assumptions of whoever wrote it.

**Closes it:** any Postgres at all — the code contains nothing specific to any
provider. `node scripts/verify-sql-connection.mjs "postgres://…"` runs the
check through the product's own crate and then greps everything POID persists
for the credential.

---

## Deliberately not built

### `poid.ai` and `poid.mcp`

Deferred to **M12** when the milestone was split, so that the credential
boundary could be reviewed as a smaller diff before the surface grew. The spec
text for both is already normative (`RUNTIME-API.md` §5, §6); only the
implementation is outstanding, including OAuth for MCP, which is its own
milestone-sized piece.

### Five of the seven Connection kinds

SPEC §7.2.1 defines `kv · sql · docs · files · ai · mcp · net · sync`.
Configurable today: **`sql` and `net`**. `ConnectionConfig` has two variants,
and the hub offers two options, because a shape invented before its use is
usually the wrong shape.

### `poid.db.docs` over a Connection

The document store's queries use SQLite's JSON functions and would have to be
translated to Postgres. Until they are, the reader offers a document-store
application **no connection at all** rather than one that fails on its third
query — `connection_choices` returns an empty candidate list for any
requirement other than `sql`.

One rough edge in that path: it reports the binding as `"local"` when the user
was never actually asked. The effect is right (the data stays local and the
badge says so) but the DTO says something that is not true, and a future reader
of that code will be misled.

### Postgres column types

Mapped: `bool`, `int2/4/8`, `float4/8`, the text family, `json`/`jsonb`,
`uuid`, `bytea`. Anything else reads as `null` rather than failing the whole
query, so one exotic column does not make `SELECT *` unusable. Notably absent:
`numeric` (its binary form needs a decimal crate) and the date/time family.

**Fix:** add `Type` arms in `column_to_json`. Mechanical.

---

## Decisions worth revisiting, not defects

- **The release profile, and an open question about binary size.**

  | Studio binary | Size |
  |---|---|
  | pre-M11, Cargo defaults | 9.36 MiB |
  | post-M11, Cargo defaults | 18.44 MiB |
  | post-M11, LTO + strip + `panic = "abort"` | 10.10 MiB |
  | **post-M11, LTO + strip — what ships today** | **16.16 MiB** |

  M11's network and database stacks roughly doubled the binary. A
  `[profile.release]` with LTO, one codegen unit and `strip` recovers most of
  it, at an appreciable build-time cost.

  **`panic = "abort"` is worth 6.06 MiB — far more than expected — and it is
  currently off.** The trade is not technical: with `abort`, a panic anywhere
  kills the process immediately, so `poid.app.onSuspend` never runs and the
  user loses the unsaved document in that window. Without it, the thread
  unwinds, the application survives, and the data can be saved. 6 MiB against
  losing someone's work seemed the wrong way round, so unwinding stayed — but
  16.16 MiB dents the product's "~10 MB, not 120 MB" claim badly enough that
  this deserves a deliberate decision rather than a default.

  Untried middle grounds: `opt-level = "s"`/`"z"`, and putting the Postgres
  driver behind a feature flag so a "reader-only" build ships without it.

  Note also that the 9.36 MiB baseline predates the profile, so M11's true
  cost is somewhere between +6.80 and +9.07 MiB rather than exactly either.
  The next milestone should take its baseline under the shipped settings.
- **The consent model is still one Run/Cancel.** So `net_fetch` derives the
  approved origin set from the manifest rather than from a recorded per-origin
  grant. When per-permission toggles arrive, the approved set should arrive
  with them; there is a comment at the derivation site.
- **The address policy does not apply to `sql` connections.** `poid.net`
  validates addresses because the *application* picks the destination; a SQL
  connection's destination is picked by the user, once. Refusing private ranges
  there would break "my Postgres is on my intranet" to stop an attack that
  needs the user to attack themselves.
