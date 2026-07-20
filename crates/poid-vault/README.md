# poid-vault

The POID vault: managed storage keyed by `instance.id`, built on a CRDT so
that offline editing on two devices merges losslessly and synchronisation
(M12) is a transport problem, not a storage rewrite (SPEC §6.5).

## Why a CRDT, not a JSON blob

This is the load-bearing decision of the storage layer. A last-write-wins
blob loses data the moment the same memory is edited on two devices while
offline — and retrofitting merge semantics later means rewriting the engine
and migrating every file. Automerge gives us conflict-free merge from day
one, with both a Rust core (desktop, CLI) and a WASM binding (Web Reader),
so one engine serves every platform.

## Modules

| Module | What it is | Platforms |
|---|---|---|
| `doc` | The document engine: slots, atomic kv values, `merge`, the operation log (`heads`/`changes_since`/`apply_changes`). Platform-pure. | native + wasm |
| `protect` | `protected` storage (SPEC §9.2): AES-256-GCM + Argon2id, self-describing envelope, fresh nonce per write. Randomness injected by the caller. | native + wasm |
| `store` | The desktop file store: one atomically-written Automerge file per instance, per-POID quota enforcement. | native (`fs`) |
| `index` | The instance index and the copy-detection state machine (SPEC §6.3): Register / Open / Moved / Copy, as a pure function over injected filesystem answers. | native (`fs`) |
| `convert` | Storage-mode conversion `embedded ↔ vault` (SPEC §6.1), with a canonical export so a round trip is byte-identical. | native (`fs`) |

The `fs` feature (default) carries everything that touches a filesystem and
pulls in `poid-core`. The `wasm` feature routes Automerge's actor-id
randomness through the browser and drops the filesystem modules — that build
is what `poid-wasm` exposes to the Web Reader.

## Design decisions worth knowing

- **Atomic values.** Each kv value is stored as one serialized JSON string,
  not a nested Automerge structure. `poid.db.kv.set` promises atomic
  replacement; a field-level merge of two concurrent same-key writes would
  invent objects the application never wrote. So a same-key conflict resolves
  to one written value (both replicas agree which), while different-key
  writes — the common case, and the two-device DoD — merge losslessly.
- **The whole document merges, i.e. all slots at once.** Instance memory is
  one unit; there is no per-slot merge.
- **Merges are quota-exempt.** Refusing a merge would drop the other
  device's writes, which is exactly what a CRDT must never do.
- **Copy detection covers vault-keyed memory only.** An embedded file's data
  travels inside it, so a copy is naturally independent — there is nothing to
  contend for.

## Tests

`cargo test -p poid-vault` covers the two-device merge, the copy-detection
state machine, byte-identical mode conversion, quota enforcement, and the
encryption round trip (including wrong-passphrase and tamper rejection).
