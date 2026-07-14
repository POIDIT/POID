# POID — Engineering Conventions

*Attach this file to every AI coding session.*

---

## Language

- **All code, comments, commit messages, docs, issues, and identifiers: English.** POID is intended to become an international standard. A Polish-language codebase cannot become one.
- Conversation with the maintainer: Polish.

## Rust (`crates/`)

- Edition 2021+. `rustfmt` + `clippy -D warnings` in CI.
- **No `unwrap()` / `expect()` outside tests.** Errors via `thiserror`; the CLI uses `anyhow` at the top level only.
- Public API is documented with `///`. `#![deny(missing_docs)]` on library crates.
- `poid-core` **MUST NOT** depend on Tauri, on any UI crate, or on the network. It is pure logic: bytes in, structures out. It must be usable from the CLI, the desktop app, and (via WASM) the browser.

## TypeScript (`packages/`, `apps/`)

- `strict: true`. No `any` — use `unknown` and narrow.
- ESM only. No CommonJS.
- Formatting/linting: Biome (fast, single tool). Not ESLint+Prettier.
- `poid-sdk` (the in-sandbox SDK) has **zero dependencies** and must be under 10 KB minified. It ships inside every POID's runtime context.

## Testing

- Every crate/package has unit tests. Every bug fix starts with a failing test.
- `poid-core` and `poid-broker` require **property-based tests** (`proptest`) for container parsing and permission checks — these are the code paths an attacker controls.
- The conformance suite (`spec/conformance/`) is the source of truth. If the spec and the code disagree, **one of them is a bug** — decide which, fix that one, never quietly diverge.

## Security rules (non-negotiable)

1. Never execute native code from a container.
2. Never pass a credential into the sandbox.
3. Never trust a scope identifier sent by the sandbox (`instanceId`, `slot`, `connectionId`). Derive scope from the window.
4. Never widen CSP without a manifest declaration **and** a user grant.
5. Never add a bypass "just for development". Use a feature flag that is compiled out of release builds.
6. Any change to `crates/poid-broker/` requires an explicit security review note in the PR.

Read `docs/SECURITY.md` before touching the broker, the sandbox, or the container parser.

## UX rules (non-negotiable)

1. **No terminal is ever shown to a user.** Build, conversion, and save show a progress bar. The terminal exists in Studio only as a deliberately-opened developer tool, like Chrome DevTools.
2. Opening a `.poid` **MUST NOT** open the Studio hub. Reader windows and Studio windows are separate.
3. Every destructive or irreversible operation is confirmed, and undoable where possible.
4. Errors are written for a person, not a developer. No stack traces in the UI.

## Git

- Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
- One milestone = one branch = one PR.
- Tag each completed milestone: `m01-core`, `m02-cli`, …

## Definition of Done (applies to every milestone)

- [ ] Compiles with zero warnings
- [ ] Tests pass, including the conformance suite where applicable
- [ ] `README.md` in the package/crate explains what it is and how to run it
- [ ] No `TODO` without a linked issue
- [ ] The maintainer can run it and see the promised result
