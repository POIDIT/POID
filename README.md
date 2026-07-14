# POID

**An open file format that makes an application behave like a document.**

POID (*Protocol for Opening Interactive Documents*) is an open container format
(`.poid` — a ZIP with a defined structure and a `manifest.json`). One file
contains an application, its data, and all of its dependencies. Double-click
opens it in its own desktop window. It can be emailed and it works offline.
There is no installation, no terminal, no build step, no server, and no
account. It is safe to open a POID from a stranger, because a POID cannot
contain machine code — it runs only JavaScript and WebAssembly, inside a
sandboxed browser engine, with no network and no credentials unless the user
grants them.

## The three rules

1. **No native code in a container. Ever.** This is the foundation of the
   safety claim, not a temporary limitation.
2. **The user never sees a terminal.** Building, converting and saving are
   silent operations with a progress bar.
3. **Credentials never enter the sandbox.** The application calls
   `poid.db.sql(...)`; the broker holds the key and executes the query.
   A malicious POID has nothing to steal.

## Repository map

| Path | What it is |
|---|---|
| `crates/poid-core` | container logic: read / write / validate `.poid` (Rust) |
| `crates/poid-vault` | Automerge CRDT storage, slots, instance identity (Rust) |
| `crates/poid-broker` | permission enforcement, Data Engine, Connections (Rust) |
| `crates/poid-cli` | the `poid` command-line tool (Rust) |
| `packages/poid-sdk` | the `window.poid` API injected into the sandbox (TS) |
| `packages/poid-host` | host side of the postMessage bridge (TS) |
| `packages/poid-toolchain` | esbuild-wasm build, Standard Library, Resolver (TS) |
| `packages/poid-ui` | shared UI: Reader chrome, consent dialog (TS) |
| `packages/poid-web` | Web Reader — static, 100% client-side SPA (TS) |
| `apps/studio` | POID Studio — the one executable (Tauri v2) |
| `apps/cloud` | POID Cloud — Cloudflare Workers services |
| `apps/mcp-server` | MCP server so AI agents can emit `.poid` files |
| `spec/` | the format specification, JSON Schema, conformance suite |
| `docs/` | spec + architecture + security + runtime API (working copies) |
| `examples/` | example POIDs (notepad, kanban, survey, …) |

## How to build

Prerequisites: [Rust](https://rustup.rs) (stable) and [pnpm](https://pnpm.io) with Node 22+.

```
# Rust workspace
cargo build && cargo test

# TypeScript workspace
pnpm install && pnpm build && pnpm test
```

## Documentation

- [docs/SPEC.md](docs/SPEC.md) — the POID format specification (normative)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the software that implements it
- [docs/SECURITY.md](docs/SECURITY.md) — threat model and security requirements
- [docs/RUNTIME-API.md](docs/RUNTIME-API.md) — the `window.poid` API surface
- [CONVENTIONS.md](CONVENTIONS.md) — engineering conventions
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to contribute

## License

- Code: [Apache-2.0](LICENSE)
- Specification: [CC-BY-4.0](spec/LICENSE)
- The POID name and logo are trademarks of the maintainer — anyone may
  implement the format; only conformant implementations may use the name.
  See [TRADEMARK.md](TRADEMARK.md).
