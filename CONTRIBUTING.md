# Contributing to POID

Thank you for contributing. Start by reading:

1. [README.md](README.md) — what POID is and the three rules
2. [CONVENTIONS.md](CONVENTIONS.md) — engineering conventions (binding)
3. [spec/SPEC.md](spec/SPEC.md) — the format specification
4. [spec/SECURITY.md](spec/SECURITY.md) — required before touching the broker,
   the sandbox or the container parser

## Ground rules

- **Follow the spec.** If the spec is unclear or wrong, open an issue and
  propose an amendment — do not silently diverge. If the spec and the code
  disagree, one of them is a bug; decide which and fix that one.
- **All code, comments, commits, docs and identifiers are in English.**
- **Conventional Commits**: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- One milestone = one branch = one PR. Completed milestones are tagged.

## Security review requirement

> **Any change to `crates/poid-broker/` requires an explicit security review
> note in the PR description.** The note must state what privileged surface
> the change touches, why it is safe, and which of the security rules in
> `CONVENTIONS.md` apply. PRs touching the broker without this note will not
> be merged.

The broker is the single gate through which every privileged operation passes.
The non-negotiable security rules (from `CONVENTIONS.md`):

1. Never execute native code from a container.
2. Never pass a credential into the sandbox.
3. Never trust a scope identifier sent by the sandbox — derive scope from the window.
4. Never widen CSP without a manifest declaration **and** a user grant.
5. Never add a bypass "just for development".

## Building

```
# Rust workspace
cargo build && cargo test

# TypeScript workspace
pnpm install && pnpm build && pnpm test

# Lint / format
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
pnpm lint
```

CI runs exactly these checks on every push and PR; all of them must be green.

## Tests

- Every crate and package has unit tests. Every bug fix starts with a failing test.
- `poid-core` and `poid-broker` require property-based tests (`proptest`) for
  container parsing and permission checks — these are the code paths an
  attacker controls.
- The conformance suite (`spec/conformance/`) is the source of truth for the
  format. `node scripts/validate-schema.mjs` checks the manifest schema
  against it.

## Licensing of contributions

Code contributions are accepted under Apache-2.0; spec contributions under
CC-BY-4.0 (see [TRADEMARK.md](TRADEMARK.md) for how the name is protected).
