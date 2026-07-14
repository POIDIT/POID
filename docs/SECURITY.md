# POID — Threat Model and Security Design

> If this document is wrong, the format is dead. A document format that can hurt you is not a document format.

---

## 1. The claim we must be able to defend

> **"You can safely open a POID from a stranger."**

Everything in the security design exists to make that sentence true. Every feature request must be tested against it.

---

## 2. The foundational constraint

> **A POID cannot contain machine code.**

This is not a limitation we intend to lift later. It is the *reason* the claim above holds.

The moment POID can execute native binaries:
- `.poid` becomes an executable → antivirus blocks it, **correctly**
- "open a POID from a stranger" becomes "install a virus from a stranger"
- we are competing with Flatpak, AppImage and MSIX — formats that have done this for 15 years and do it better

Consequence, stated plainly: GIMP, FreeCAD and LibreChat **cannot** be wrapped in a POID. Image editors, CAD tools and AI clients **can** be built as POIDs, targeting the browser engine. Wrapping ≠ porting.

---

## 3. Threat model

### Assets
1. The user's files and OS
2. The user's credentials (API keys, DB passwords, OAuth tokens)
3. The user's data in other POIDs
4. The user's network position (LAN, intranet)
5. The user's attention and consent

### Adversary
A malicious POID, delivered by email, download, or a compromised registry. Assume the attacker fully controls the container's contents.

### Attack surface, and how each is closed

| Attack | Mitigation |
|---|---|
| Ship a native payload | Container validation rejects executable magic bytes; ZIP entries are scanned |
| Zip bomb | Decompression ratio cap (100:1) + absolute size cap |
| Path traversal on extraction | Reject `..`, absolute paths, drive letters, symlinks; never extract to disk unsandboxed |
| Escape the sandbox | `<iframe sandbox="allow-scripts">`, opaque origin, no `allow-same-origin` |
| Exfiltrate data over the network | CSP `connect-src 'none'` by default. Not a policy — a browser-enforced impossibility |
| Steal API keys | **Credentials never enter the sandbox.** The broker holds them and executes on the app's behalf |
| Read another POID's data | Vault entries are keyed by `instance.id`; the broker scopes every call |
| Read another slot | Slot switching is a reader operation; the app cannot request a slot |
| Read the host filesystem | No path-based FS API exists. Only `poid.files.open()` → native dialog → user picks |
| Spawn a process | No API exists. MCP `stdio` transport is unavailable to applications |
| SSRF into the user's LAN | Network allowlist is origin-scoped; broker blocks private ranges unless explicitly policy-allowed |
| Exhaust CPU / RAM / disk | Watchdog, quota, visible "Stop this application" control |
| Consent fatigue / dark patterns | Consent screen is reader-drawn, outside the sandbox. The app cannot style, cover, or fake it |
| Tamper after signing | `integrity` hashes + optional Ed25519 signature. Any content change re-triggers consent |
| Impersonate a publisher | Signed publishers, verified badge, trust-on-first-use with pinning |

---

## 4. The broker — the single most important component

**Everything privileged passes through one gate.**

```
sandboxed app          Reader window            Core (Rust)
──────────────         ─────────────            ───────────
poid.db.sql(q)  ──postMessage──►  IPC  ──►  BROKER
                                              ├─ Is `sql` declared in the manifest?
                                              ├─ Did the user grant it?
                                              ├─ Is the quota intact?
                                              ├─ Attach credentials (from keychain)
                                              ├─ Execute
                                              └─ Return ONLY the result
                       ◄────────────────────  rows
```

**Normative invariants:**

1. The sandbox **never** receives a credential — not in a response, not in an error, not in a stack trace.
2. The broker **never** trusts a parameter that identifies scope (`instanceId`, `slot`, `connectionId`). It derives scope from the window that sent the message.
3. Every broker entry point is fail-closed: unknown method → reject.
4. Broker code is the smallest, most-reviewed, most-tested code in the repository.

> Any pull request touching `crates/poid-broker/` requires explicit security review. Put this in `CONTRIBUTING.md` and mean it.

---

## 5. Consent

**Preview mode is mandatory on first open.** The reader shows, outside the sandbox:

```
┌──────────────────────────────────────────────┐
│  Kanban                              v1.0.0  │
│  by Jane Doe        ⚠ Unsigned publisher     │
│                                              │
│  This application requests:                  │
│    • Store data in this file                 │
│                                              │
│  It does NOT request:                        │
│    • Internet access                         │
│    • Access to your files                    │
│    • Access to your credentials              │
│                                              │
│  [ View source ]   [ Cancel ]   [ Run ]      │
└──────────────────────────────────────────────┘
```

- Consent is recorded per `app.id` + content hash.
- Any content change → consent is re-requested.
- Widened permissions on update → consent is re-requested.
- Listing what the app *does not* request is a deliberate design choice: it makes the safe case visibly safe, which is what builds trust in a format.

---

## 6. What the user can always do

These are guarantees, not features. They are what makes POID trustworthy rather than merely convenient.

- **Extract my data**, from any POID, with any ZIP tool. (Unless *they* encrypted it.)
- **Read the source**, because there is no compiled binary to hide behind.
- **Work fully offline**, forever.
- **Leave.** Convert `vault` → `embedded` and POID Cloud becomes irrelevant.
- **Not be tracked.** Zero telemetry by default. No account required. Ever.

---

## 7. Encryption (`storage.protected`)

- AES-256-GCM, key derived via Argon2id (recommended: m=64 MiB, t=3, p=4).
- Nonce is unique per write. Never reused.
- The passphrase is never stored, never sent, never recoverable.
- **The reader MUST tell the user the truth:** *"Send without data" is strictly safer than "Send encrypted."* Absent data cannot leak.

---

## 8. Supply chain

- Standard Library dependencies: pinned versions, checksums committed, reproducible builds.
- Resolver downloads: checksum-verified against the registry, recorded in `runtime.bundled_deps`.
- Engines (Pyodide, wa-sqlite): checksum-verified, versions pinned, fetched only by Studio with consent.
- Studio releases: signed and reproducible. Publish the build hashes.

---

## 9. Rules for reviewers

Reject any change that:

- introduces native code execution from a container
- passes a credential into the sandbox
- widens CSP without an explicit, user-granted manifest declaration
- lets an application choose its own storage scope
- lets an application suppress, style, or bypass the consent screen
- adds a "just this once" bypass to any of the above

Every one of these has looked reasonable to somebody, in some pull request, in some project. That is exactly how document formats become malware vectors.
