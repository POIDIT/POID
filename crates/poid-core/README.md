# poid-core

Container logic for the POID format: reading, writing and validating `.poid` files
(ZIP structure, `manifest.json`, integrity, prohibited-content checks — SPEC §2–§3).

Pure logic: bytes in, structures out. No Tauri, no UI, no network. Used by the CLI,
POID Studio and (via WASM) the Web Reader.

```
cargo test -p poid-core
```

Status: stub (M00). Container parsing lands in a later milestone.
