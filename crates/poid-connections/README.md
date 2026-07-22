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
touches the operating system — the keychain, the registry file — is here, in a
crate whose entire job is to keep those two things apart.

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
