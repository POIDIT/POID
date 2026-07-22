# poid-broker

The broker's **policy**: permission enforcement, Connections and the network
rules every privileged operation from a sandboxed application is measured
against (SPEC §7, §9).

**Any change to this crate requires an explicit security review note in the PR.**
Read `spec/SECURITY.md` before touching it.

```
cargo test -p poid-broker
```

## What is here, and what is deliberately not

This crate decides; it never acts. It performs no IO, opens no sockets, reads
no keychain, and — importantly — **holds no secret**. Every public item is a
pure function over data, which is what makes the security-critical decisions
exhaustively testable, including with `proptest` over inputs an attacker
chooses.

The mechanism lives elsewhere: keychain access and database drivers in
`poid-connections`, sockets and IPC in the Studio host. Keeping the two apart
means the code worth reviewing line by line is all in one place and cannot
accidentally perform an effect.

| Module | Decides |
|---|---|
| `connection` | what a configured backend is, and which storage needs each kind can serve (SPEC §7.2.1) |
| `binding` | which backend serves an application, given the manifest and the user's recorded choice (SPEC §7.2.3) |
| `network` | which origins may be reached and which addresses may be connected to (SPEC §7.2.5) |
| `scrub` | removing credentials from the diagnostics the *user* reads |
| `error` | the six codes an application may see, and nothing else (RUNTIME-API §9) |

## The two invariants worth knowing before you edit

**A `ConnectionRef` has no secret field, and must never gain one.** Code that
needs a credential asks the keychain for it by id, in the process that performs
the operation, at the moment it performs it. Code that only needs to decide
*whether* something is allowed works from the reference alone. This is the
type-level half of SPEC §7.1.

**An origin is checked; an address is connected to.** These are separate
functions on purpose. Checking a hostname and then handing the name to an HTTP
client is defeated by DNS rebinding — the name resolves public when checked and
to `169.254.169.254` when connected. Callers resolve, filter with
`usable_addresses`, and connect to an address that passed.
