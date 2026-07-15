# poid-core

Container logic for the POID format: reading, writing and validating `.poid` files
(SPEC §2–§4, §6). Pure logic — bytes in, structures out. No Tauri, no UI, no network.
Used by the CLI, POID Studio and (via WASM) the Web Reader.

## What it does (M01)

- **Manifest** — serde types for the full SPEC §3.1 manifest; unknown fields
  round-trip losslessly at every level; validation with stable error codes.
- **Reader** — `open` / `open_path` enforce, in order: the `mimetype` magic
  (first entry, STORED, exact content), manifest validation, prohibited
  content (executable magic bytes, links, path traversal — including inside
  nested archives such as Python wheels), zip-bomb limits (streamed, 100:1
  ratio + absolute budget), and type-specific rules.
- **Writer** — `PoidBuilder` + `pack` produce deterministic output: packing
  the same input twice is byte-identical. `mimetype` and `manifest.json` are
  generated, never taken from the caller.
- **Integrity** — canonical SHA-256 tree digests over `app/` and `deps/`
  (SPEC §3.3); `verify()` detects tampering. `data/` is excluded by design so
  saving user data does not invalidate consent.
- **Mutation** — `set_instance_id`, `set_data`, `clear_data`
  ("Duplicate as empty"), `convert_storage_mode`; `save_path` writes
  atomically (temp file + fsync + rename).

## Building

```
cargo test -p poid-core
cargo build -p poid-core --no-default-features --target wasm32-unknown-unknown
```

The `fs` feature (default) gates filesystem APIs; without it the crate is
pure and builds for `wasm32-unknown-unknown`.

Every rejection carries a stable, machine-readable code (`PoidError::code`),
which the conformance suite asserts on (SPEC §14). Property-based tests
guarantee the parser never panics on any input.
