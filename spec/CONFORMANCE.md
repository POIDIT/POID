# POID Conformance

**Status:** Normative companion to `SPEC.md` §14.

A specification without a test suite is an opinion. This document defines
what it means to be a conformant POID implementation and how to prove it
with the suite in `spec/conformance/`.

## 1. What a conformant implementation must do

An implementation is **conformant** when it:

1. **Opens** every container in `spec/conformance/valid/` successfully.
2. **Rejects** every container in `spec/conformance/invalid/` and can report
   the registry code (`spec/errors.md`) named by the fixture's
   `*.expected.json`.
3. Enforces the normative runtime requirements of the spec — see §4 below.

## 2. The container suite

### Layout

```
spec/conformance/
├── valid/
│   ├── minimal-html.poid
│   ├── minimal-html.expected.json      { "valid": true }
│   └── …
├── invalid/
│   ├── contains-exe.poid
│   ├── contains-exe.expected.json      { "valid": false, "code": "POID-020" }
│   └── …
└── manifest/                           JSON Schema fixtures for manifest.json
    ├── valid/…
    └── invalid/…
```

Every fixture has a sibling `<name>.expected.json`:

```json
{ "valid": true }
{ "valid": false, "code": "POID-020" }
```

### The check, precisely

For each `*.poid`, a conformant reader's verdict is the result of:

1. **Open** — enforce SPEC §2 (mimetype magic, manifest parse + §3.1
   validation, prohibited content, decompression limits with the defaults
   from `errors.md`'s referenced sections, type-specific rules).
2. **Verify integrity** — recompute the §3.3 tree digests and compare with
   the manifest (POID-031 on mismatch).
3. **Check the signature, if present** — a signature that does not verify is
   POID-050; a malformed signature file is POID-051; **no signature is not
   an error** (SPEC §9.3).

The reference implementation exposes exactly this predicate as
`poid_core::conformance_check`, and as the commands
`poid validate <file>` and `poid conformance <suite-dir>`.

### Running the suite

```
poid conformance spec/conformance          # human-readable report
poid conformance spec/conformance --json   # machine-readable report
```

Exit code 0 means 100% of fixtures passed. Any implementation, in any
language, may implement the same runner semantics: read each `*.poid`, apply
the check above, compare with `expected.json`.

### Regenerating the fixtures

Fixtures are **generated, never hand-crafted**, by a committed program:

```
cargo run -p poid-fixtures --bin gen-conformance -- spec/conformance
```

Generation is deterministic (fixed timestamps, fixed pseudo-random seed,
fixed well-known Ed25519 test key), so regenerating must produce
byte-identical files — CI enforces this. The test key seed is 32 bytes of
`0x2a` and is public by design; never use it to sign real content.

## 3. Notes for implementers

- **Read `errors.md` first.** The registry code, not the exact message, is
  the contract.
- Each invalid fixture contains exactly one violation, so check *order*
  cannot change the expected code.
- The `manifest/` subdirectory contains JSON-only fixtures for validating a
  manifest against `spec/schema/manifest.schema.json` without building
  containers — useful early in a port.
- Two container-format details are intentionally *not* yet exercised by the
  suite because the spec does not fully define them: the slot `current`
  pointer (SPEC §6.4) and the encrypted layout of `protected` storage (SPEC
  §9.2 defines the algorithm but not the on-disk framing). The `slots` and
  `protected` fixtures test container shape only. Spec amendments will
  follow with the corresponding milestones.

## 4. The naming rule

The conformance suite proves *container-level* conformance. A **reader** —
software that executes POID applications — must additionally enforce, at
run time:

- **Isolation** (SPEC §5.2): sandboxed execution context, a dedicated
  reader-controlled origin isolated from the host (§5.2.1), CSP with
  `connect-src 'none'` by default;
- **Credential isolation** (SPEC §7.1): the application never receives a
  credential, in any form, ever — and neither does any other browser-engine
  context, including the reader's own UI;
- **Connections** (SPEC §7.2), for a reader that offers them: the application
  cannot name a provider, the user can always decline and keep data local,
  backend errors are scrubbed, and outbound requests connect only to a
  validated address;
- **Consent** (SPEC §9.1): preview mode before first execution, consent
  recorded per `app.id` + content hash, re-triggered on change.

These cannot be proven by a fixture suite; they are normative requirements.

A reader that offers no Connections at all is still conformant — §7.2 binds
only implementations that do. What is **not** conformant is offering them
with a weaker secret store than §7.2.2 requires.

> A reader that passes the container suite but does not enforce §5.2, §7.1
> and §9.1 **MUST NOT** be described or marketed as a POID reader
> (SPEC §14, TRADEMARK.md). This is the sentence that keeps "you can safely
> open a POID from a stranger" true.
