# poid-broker

The broker: permission enforcement, the Data Engine (kv / sql / docs / remote) and
Connections (OS keychain, credential isolation). Every privileged operation from a
sandboxed application passes through this crate (SPEC §7, §9).

**Any change to this crate requires an explicit security review note in the PR.**
Read `docs/SECURITY.md` before touching it.

```
cargo test -p poid-broker
```

Status: stub (M00). Permission checks land in a later milestone and require
property-based tests (`proptest`).
