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

- **The release profile.** M11 roughly doubled the Studio binary on Cargo's
  defaults (9.36 → 18.44 MiB). Adding a `[profile.release]` with LTO, one
  codegen unit and `strip` brought it to **10.10 MiB**, so the product's
  "~10 MB" claim survives. Build time rises appreciably.

  Two caveats on that number. It was measured with `panic = "abort"`, which
  was then **dropped**: it saves size but skips `onSuspend`, so an application
  holding unsaved user data would lose it on a panic instead of getting a
  chance to flush. The shipped profile is therefore slightly larger than 10.10
  MiB and the exact figure was not captured. And the 9.36 MiB baseline predates
  the profile, so part of the apparent 0.74 MiB growth is the profile itself —
  M11's true cost is somewhere between +0.74 and +9.07 MiB.

  **Worth doing:** measure the shipped profile once, and record it, so the next
  milestone has a baseline that was taken under the same settings.
- **The consent model is still one Run/Cancel.** So `net_fetch` derives the
  approved origin set from the manifest rather than from a recorded per-origin
  grant. When per-permission toggles arrive, the approved set should arrive
  with them; there is a comment at the derivation site.
- **The address policy does not apply to `sql` connections.** `poid.net`
  validates addresses because the *application* picks the destination; a SQL
  connection's destination is picked by the user, once. Refusing private ranges
  there would break "my Postgres is on my intranet" to stop an attack that
  needs the user to attack themselves.
