# poid-connections

Where a POID Connection's credential actually lives, and where the registry of
configured connections is kept (SPEC §7.2).

**This crate handles real secrets.** Changes to it belong in the same security
review as `poid-broker`. Read `spec/SECURITY.md` first.

```
cargo test -p poid-connections
```

## The split with poid-broker

`poid-broker` decides; this crate acts. The broker never holds a credential and
never performs an effect, so it can be reviewed as pure logic. Everything that
touches the operating system — the keychain, the registry file, sockets — is
here, in a crate whose entire job is to keep those two things apart.

| Module | Does |
|---|---|
| `secret` | the OS credential store, behind one trait |
| `store` | the registry: metadata on disk, credentials in the keychain |
| `bindings` | which connection serves which document (SPEC §7.2.3) |
| `dsn` | splits a Postgres connection string into what may be shown and what may not |
| `http` | the brokered `poid.net` request (SPEC §7.2.5) |
| `sql` | the `sql` connection kind, over the Postgres wire protocol |

## How a credential is kept apart from everything else

The registry file holds `{id, label, config}`. The keychain holds the secret,
under service `dev.poid.studio`, account = connection id. They meet in exactly
one place: `ConnectionStore::upsert`, which takes the credential as a separate
argument, derives the displayable parts, writes those to the file and hands the
secret to the `SecretStore`.

That means the usual way this kind of bug appears — someone adds a field to a
struct that later gets serialised — cannot happen here. `ConnectionRecord` has
no secret field. Neither does `poid_broker::ConnectionRef`. `NewConnection` is
the only type that carries one, and it exists solely as a function argument.

For a SQL connection the **whole DSN** is the secret, and `SqlTarget`
(host, port, database, role) is derived for display. Storing host and database
separately would create a second path to keep in sync for no benefit, and a
partially-stored secret is one somebody eventually reassembles.

## Platforms, and the deliberate absence of a fallback

| Platform | Store |
|---|---|
| Windows | Credential Manager |
| macOS | Keychain |
| Linux | Secret Service (over zbus — pure Rust, no D-Bus headers needed to build) |

A platform without one of these does not get a weaker fallback. It gets no
credentialed connections at all (SPEC §7.2.2). A secret kept somewhere the user
cannot see is a secret they cannot revoke, and they would go on believing it
was in the keychain.

`MemoryStore` is a test double behind the `test-store` feature, which `default`
does not enable, so it is compiled out of every release build (CONVENTIONS,
security rule 5). It is not a fallback and must never be wired up as one.

## The network path, and why it looks paranoid

`http::brokered_fetch` checks the allowlist, resolves the name itself,
validates every address, **connects to an address that passed**, strips the
application's `Authorization`, attaches the real credential, and repeats all of
that for each redirect hop.

The step people skip is the third one. Checking a hostname and then handing the
name to an HTTP client is defeated by DNS rebinding: the name resolves to a
public address while it is being checked and to `169.254.169.254` when it is
connected to. The check and the connection have to agree on an *address*.

Redirects are followed here rather than by `reqwest` for the same reason — a
client that follows them internally takes the second hop with none of these
checks applied.

## The SQL path

`sql::PostgresConnection` speaks the Postgres wire protocol, so `poid.db.sql`
runs real SQL. Supabase is a preset rather than an integration; Neon, RDS and a
self-hosted server are the same path.

The address policy deliberately does **not** apply here. `poid.net` validates
addresses because the *application* chooses the destination; a SQL connection's
destination is chosen by the user, once, in Studio. Refusing private addresses
would break the common case — a Postgres on your own intranet — to prevent an
attack that requires the user to attack themselves.

### Column types this build maps

`bool`, `int2`, `int4`, `int8`, `float4`, `float8`, `text`/`varchar`/`bpchar`/
`name`, `json`, `jsonb`, `uuid`, `bytea` (as Postgres's `\x` hex form).

Anything else reads as `null` rather than failing the whole query: one exotic
column should not make `SELECT *` unusable for an application that never reads
it. Extending the list is mechanical — add a `Type` arm in `column_to_json`.

### Not served by a connection yet

`poid.db.docs`. The document store's queries are SQLite-specific and
translating them to Postgres is real work that is not done. Rather than offer a
connection that fails on the third query, the reader offers a document-store
application no connection at all, and it keeps its data locally.

## Verifying against a real database

CI cannot: it needs a real server and a real credential, and putting either in
the repository is the mistake this crate exists to prevent.

```
node scripts/verify-sql-connection.mjs "postgres://user:pass@host:5432/db"
```

It runs through this crate rather than a second implementation, and greps
everything POID persists for the credential afterwards.
