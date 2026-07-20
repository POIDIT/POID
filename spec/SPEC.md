# POID Format Specification

**Version:** 1.0.0-draft.1
**Status:** Draft
**Media type:** `application/vnd.poid+zip`
**File extension:** `.poid`
**License:** CC-BY-4.0

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** in this document are to be interpreted as described in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

---

## 1. Introduction

### 1.1 Purpose

POID (*Protocol for Opening Interactive Documents*) is an open container format that makes an application behave like a document.

A single `.poid` file contains an application, its data, and all of its dependencies. It opens by double-click into its own window. It can be emailed, copied, archived, and version-controlled like any other file. It requires no installation, no terminal, no build step, no server, and no network connection.

### 1.2 Design principles

1. **A POID is a document, not an installer.** It MUST be safe to open a POID received from an untrusted sender.
2. **No native code, ever.** A POID MUST NOT contain machine code, executables, or shared libraries. This is the foundation of principle 1 and is not negotiable.
3. **Self-contained and deterministic.** A POID MUST run offline. Opening the same POID ten years from now MUST produce the same result.
4. **The reader provides engines; the file provides everything else.** The container carries application dependencies. The reader carries language runtimes (JS, WASM engines).
5. **Deny by default.** Network access, filesystem access, and credentials are unavailable unless declared in the manifest AND granted by the user.
6. **The user's data belongs to the user.** Data MUST be extractable without proprietary tooling.

### 1.3 Non-goals

POID explicitly does **not** support, and will never support:

- native binaries, `.exe`, `.dll`, `.so`, `.dylib`
- system processes, port listeners, servers
- Docker, GPU compute, model training
- package installation on the consumer's machine (`npm install`, `pip install`)

Wrapping an existing native application (e.g. GIMP, FreeCAD, LibreChat) inside a POID is out of scope. **Porting** such an application to the browser engine is in scope and encouraged.

---

## 2. Container

### 2.1 Physical format

A POID file **MUST** be a ZIP archive as defined by ZIP APPNOTE 6.3.x.

The first entry **MUST** be a file named `mimetype`, stored **uncompressed** (STORED, method 0), with no extra field, whose content is exactly the ASCII string:

```
application/vnd.poid+zip
```

with no trailing newline. This mirrors the EPUB convention and allows type detection by reading the first ~60 bytes (magic bytes).

Implementations **MUST** reject archives where the first entry is not a conformant `mimetype`.

### 2.2 Directory layout

```
example.poid                (ZIP)
├── mimetype                REQUIRED  uncompressed, first entry
├── manifest.json           REQUIRED  the contract
├── app/                    REQUIRED for type=app|workspace — built, runnable output
│   ├── index.html
│   ├── main.js
│   └── style.css
├── src/                    OPTIONAL  original sources (for inspection / editing)
├── assets/                 OPTIONAL  icons, images, fonts
├── data/                   OPTIONAL  application state (see §6)
│   └── store.json
├── slots/                  OPTIONAL  multiple named states (see §6.4)
│   ├── project-a/
│   └── project-b/
├── deps/                   OPTIONAL  bundled runtime dependencies (wheels, wasm)
├── migrations/             OPTIONAL  data schema migrations (see §12)
├── apps/                   REQUIRED for type=workspace — nested POIDs (§4.3)
└── signature/              OPTIONAL  Ed25519 signature (see §9.3)
```

### 2.3 Prohibited content

An implementation **MUST** reject a POID that contains any entry that is:

- a native executable or library (detected by magic bytes: `MZ`, `\x7fELF`, `\xfe\xed\xfa`, `\xce\xfa\xed\xfe`, `\xcf\xfa\xed\xfe`, `!<arch>`)
- a symbolic link or hard link
- a path traversal (`..`, absolute paths, drive letters)
- a nested archive that itself contains prohibited content

Implementations **MUST** enforce a decompression ratio limit (RECOMMENDED: 100:1) and an absolute uncompressed size limit, to prevent zip-bomb attacks.

---

## 3. Manifest

`manifest.json` **MUST** be valid UTF-8 JSON. Unknown fields **MUST** be preserved on round-trip and **MUST NOT** cause rejection (forward compatibility).

### 3.1 Full schema

```jsonc
{
  "poid": "1.0",                          // REQUIRED, spec version

  "type": "app",                          // REQUIRED: "app" | "data" | "workspace"

  "app": {                                // REQUIRED for type=app|workspace
    "id": "com.example.kanban",           // REQUIRED, reverse-DNS, stable across versions
    "name": "Kanban",                     // REQUIRED, human-readable
    "version": "1.0.0",                   // REQUIRED, semver
    "author": "Jane Doe",                 // OPTIONAL
    "description": "A simple kanban board", // OPTIONAL
    "license": "MIT",                     // OPTIONAL, SPDX identifier
    "icon": "assets/icon.svg",            // OPTIONAL, path within container
    "window": {                           // OPTIONAL, reader window hints
      "width": 1100,
      "height": 750,
      "min_width": 480,
      "min_height": 360,
      "resizable": true
    }
  },

  "instance": {                           // REQUIRED for type=app|workspace
    "id": null                            // null until first open; see §6.3
  },

  "draft": false,                         // OPTIONAL, default false. true = opens in Studio, not Reader

  "runtime": {                            // REQUIRED for type=app|workspace
    "profile": "web",                     // REQUIRED: "web" | "web+python" | "web+sql" | ...
    "engines": {                          // OPTIONAL, semver ranges
      "pyodide": ">=0.26 <0.28"
    },
    "bundled_deps": [                     // OPTIONAL, audit trail
      "react@18.3.1",
      "recharts@2.12.0"
    ],
    "toolchain": {                        // OPTIONAL, reproducibility record
      "builder": "poid-cli@1.0.0",
      "esbuild": "0.24.0"
    }
  },

  "entry": "app/index.html",              // REQUIRED for type=app

  "storage": {                            // REQUIRED for type=app|workspace
    "mode": "embedded",                   // "embedded" | "vault" | "connection"
    "slots": false,                       // true = multiple named states
    "protected": false,                   // true = data encrypted at rest (§9.2)
    "quota_mb": 64,                       // OPTIONAL, requested quota
    "requires": {                         // REQUIRED if mode = "connection"
      "kind": "sql",                      // "kv" | "sql" | "docs" | "files"
      "hint": "supabase"                  // OPTIONAL, suggested provider
    }
  },

  "permissions": {                        // REQUIRED for type=app|workspace
    "network": [],                        // allowlist of origins; [] = no network
    "filesystem": "none",                 // "none" | "user-initiated"
    "clipboard": false,
    "print": false,
    "notifications": false,
    "mcp": []                             // MCP server ids the app may call
  },

  "shared_scope": [],                     // OPTIONAL, type=workspace only (§10)

  "data_ref": {                           // REQUIRED for type=data (§11)
    "app_id": "com.example.survey",
    "app_version": "1.0.0",
    "schema": "responses/v1"
  },

  "integrity": {                          // REQUIRED
    "algo": "sha256",
    "app": "a3f5...",                     // hash of app/ tree
    "deps": "b81c..."                     // hash of deps/ tree
  }
}
```

### 3.2 `app.id` vs `instance.id` — critical distinction

| Field | Answers | Lifetime | Used for |
|---|---|---|---|
| `app.id` | *What program is this?* | Stable across versions and copies | updates, permissions, icons, registry |
| `instance.id` | *Which copy is this?* | Unique per file instance (UUIDv4) | binding vault memory to this file |

`instance.id` **MUST** be `null` in a freshly packed POID. The reader **MUST** generate a UUIDv4 and write it into the manifest on first open. See §6.3 for copy detection.

### 3.3 Integrity computation (normative)

The `integrity` digests are computed per tree: `integrity.app` over `app/`, `integrity.deps` over `deps/`.

1. Take every file in the tree, identified by its **full container path** (e.g. `app/index.html`), sorted by byte-wise path order.
2. For each file compute `SHA-256( path ‖ 0x00 ‖ content )`.
3. The tree digest is the SHA-256 of the **concatenation of those per-file digests**, encoded as 64 lowercase hex characters.

Hashing a list of per-file digests — rather than one concatenated stream — makes file boundaries unambiguous: no combination of paths and contents can collide with a different combination.

A tree with no files has no digest; the corresponding field **MUST** be omitted.

`data/`, `slots/`, `manifest.json` and `signature/` are deliberately **excluded** from the digests: consent is keyed to the application hash (§9.1), so saving user data or assigning `instance.id` **MUST NOT** invalidate consent.

---

## 4. Container types

### 4.1 `type: "app"`
The default. Contains a runnable application. Opens in a Reader window.

### 4.2 `type: "data"`
Contains **only data** — no `app/`, no `entry`, no code. Used for:
- survey responses returned by a respondent (offline collection)
- data import/export
- backups

`data_ref` **MUST** identify the application the data belongs to. A reader that receives a `type: data` POID **SHOULD** offer to import it into the matching application.

`type: data` POIDs **MUST NOT** contain `app/`, `src/`, or `deps/`. They are inert by construction.

### 4.3 `type: "workspace"`
Contains multiple nested POIDs under `apps/`. See §10.

A workspace **MUST** contain at least one nested POID under `apps/`; a
workspace without `apps/` is not conformant (POID-041). A workspace with an
empty shell and no applications has no meaning.

---

## 5. Runtime model

### 5.1 Execution boundary

An application **MUST** be executed by a **browser engine** (WebView / browser) inside an isolated context. Permitted execution technologies:

- JavaScript (ECMAScript)
- WebAssembly
- CSS, HTML, SVG, Canvas, WebGL, WebGPU

Nothing else. There is no other execution path.

### 5.2 Isolation requirements

A conformant reader **MUST**:

1. Load `entry` inside a **sandboxed iframe** (`sandbox="allow-scripts"`, **never** `allow-same-origin`), or an equivalent isolation primitive on platforms without iframes.
2. Apply a Content-Security-Policy that at minimum enforces:
   ```
   default-src 'self';
   connect-src 'none';
   object-src 'none';
   base-uri 'none';
   form-action 'none';
   ```
   `connect-src` **MUST** be widened only to origins explicitly listed in `permissions.network` **and** approved by the user. `script-src`/`style-src` **MUST NOT** be widened beyond the container's own dedicated origin (§5.2.1).
3. Serve container contents from a **dedicated origin the reader controls** (§5.2.1), isolated from the host: the application **MUST NOT** be able to read the container file itself, the vault, other POIDs, the host page, or the host filesystem.
4. Route **all** privileged operations through a **broker** in the reader core (see §7).

#### 5.2.1 The container's dedicated origin (synthetic origin)

A reader **MUST** serve the application and every relative subresource
(`app/main.js`, stylesheets, images, fonts, WASM) from an origin that:

- serves **only** container bytes (`ContainerServer.resolve()` in this
  repository is the single path→response authority), and
- is **isolated from the host origin** — cross-origin from the reader UI,
  the vault, and every other open instance, and never reachable by the
  application except as relative URLs within its own tree.

Two conformant realisations of this origin are used in this repository:

| Reader | Mechanism | Origin |
|---|---|---|
| Desktop (Tauri) | a custom URI-scheme protocol handler | `poid://<session>/…` |
| Web | a service worker serving a per-session scope | a reader-controlled path/origin |

> **Amendment note (M09):** through M08 this clause read *"serve container
> contents from an **opaque** origin."* An opaque origin has no name a CSP
> `script-src` can reference, so `<script src="app/main.js">` never executes —
> only inline scripts run, and a multi-file application (e.g. the `react-app`
> conformance fixture) renders its markup but not its bundle. M06 worked
> around this by inlining the whole application into one HTML document. The
> fix (issue #5) is to serve subresources from a **named, reader-controlled
> origin** so `script-src 'self'` resolves to container content and nothing
> else; isolation is then provided by that origin's separation from the host
> (cross-origin) plus `sandbox="allow-scripts"` without `allow-same-origin`,
> rather than by opaqueness. Recorded here per CONVENTIONS ("the spec and the
> code must never quietly diverge").

### 5.3 Frameworks

POID stores **build output**, not source projects. Consequently, every frontend framework whose output is HTML+CSS+JS is supported with no reader-side work: React, Vue, Svelte, Solid, Angular, Preact, Lit, htmx, Alpine, three.js, D3, and others.

The build step happens **at authoring time**, inside POID Studio or the CLI. Consumers never build anything.

### 5.4 Language runtimes (engines)

Languages other than JavaScript are supported via WebAssembly engines **provided by the reader**, never by the file:

| Profile | Engine | Notes |
|---|---|---|
| `web` | (none) | JS/WASM only. Always available. |
| `web+python` | Pyodide | Wheels for third-party packages ship in `deps/` |
| `web+sql` | wa-sqlite / PGlite | See §8 |

Readers **MUST** verify engine integrity (checksum) before use. Readers **MAY** download a missing engine from a signed registry, **with user consent**. Applications **MUST NOT** be able to trigger engine download.

#### 5.4.1 Engine manifest format — **DRAFT, proposed in M06, not yet normative**

> **Status note:** §5.4 requires checksum verification but does not define what an engine manifest looks like. M06 needed a concrete shape to implement `web+python`, so this subsection documents what was built (`engines/pyodide.json` in this repository, verified by `packages/poid-host/src/engines.ts`). It is a **proposal**, not yet a ratified part of the format — flagged here per CONVENTIONS ("the spec and the code must never quietly diverge") rather than folded into the normative text silently. The maintainer should promote it to a `MUST`, amend it, or reject it before a second independent reader implementation relies on it.

A conformant engine ships alongside an **engine manifest**, a JSON document with:

```jsonc
{
  "name": "pyodide",              // engine identifier, matches runtime.engines keys
  "version": "0.26.4",            // exact version this reader ships
  "compatible": ">=0.26 <0.27",   // the runtime.engines range this build satisfies
  "source": "https://…",          // where the reader's build obtained the engine (informative)
  "sourceSha256": "…",            // sha256 of the upstream archive (build-time provenance)
  "files": {                      // every file the engine loader executes or reads
    "pyodide.js": "…sha256…",
    "pyodide.asm.wasm": "…sha256…"
    // …
  }
}
```

**Verification (normative under this proposal):** before executing any engine file, the reader **MUST** fetch and checksum **every** entry in `files` and compare it against the pinned value. A reader that begins executing before every file is verified does not satisfy §5.4's checksum requirement — verify-then-run, not run-then-check.

**Version matching:** the reader compares its engine manifest's `version` against the application manifest's `runtime.engines.<name>` range using standard semver range semantics (comparators, `^`, `~`, `||`). An unparseable range or version **MUST** be treated as a mismatch, never as a silent pass.

**Provenance vs. runtime trust:** `source` / `sourceSha256` record how the reader's *build* obtained the engine (useful for audits and reproducible reader builds) and are not consulted at load time — only `files` gates execution. This mirrors the Standard Library catalog (§5.2): a build-time provenance record plus a runtime-verified checksum set.

This repository's reference instance is `engines/pyodide.json`; `scripts/fetch-pyodide.mjs` reproduces it deterministically from the pinned `source`.

---

## 6. Storage

### 6.1 Modes

| Mode | Data lives in | Copy semantics | Sync | Sharing |
|---|---|---|---|---|
| `embedded` | `data/` inside the container | Copying the file copies the data | via file sync (Drive, Dropbox) | Send the file → recipient sees data |
| `vault` | Reader's managed store, keyed by `instance.id` | Copies are detected (§6.3) | via CRDT sync (§6.5) | Send the file → recipient sees **no** data |
| `connection` | External backend the user configured | N/A | native to the backend | Data never travels with the file |

`embedded` is the **default** and is **RECOMMENDED** for documents intended to be shared.

Readers **MUST** support conversion between all three modes, in both directions, on any machine.

### 6.2 Data format

In `embedded` mode, `data/store.json` **MUST** be human-readable UTF-8 JSON unless `storage.protected` is `true`.

> **Rationale:** archival longevity, DLP scannability for enterprises, and the guarantee that a user can recover their data with nothing but a ZIP tool. Confidentiality is provided by `protected` (§9.2), not by obscurity.

### 6.3 Instance identity and copy detection

Because `Ctrl+C` produces a byte-identical file, no identifier stored in the file can, by itself, distinguish a copy from the original. Readers **MUST** implement the following algorithm.

On open, if `instance.id` is `null`:
> Generate UUIDv4, write it to the manifest, register `{instance_id → (path, file_hash)}` in the vault index. Proceed.

On open, if `instance.id` is set and present in the vault index:
1. **Same path** → normal open.
2. **Different path, previous path no longer exists** → the file was moved. Update the index silently.
3. **Different path, previous path still exists** → this is a **copy**. The reader **MUST** prompt the user:
   - **Fork** *(default)* — assign a new `instance.id`, start with an independent memory
   - **Move** — treat as relocation, update index
   - **Share memory** — both files address the same vault entry

Readers **SHOULD** offer a **"Duplicate as empty"** operation: same `app/`, cleared `data/`, `instance.id` reset to `null`.

### 6.4 Slots (multi-memory)

When `storage.slots` is `true`, state is stored under `slots/<name>/` and a `current` pointer selects the active slot.

The **application is not aware of slots.** It calls `poid.db` as usual; the reader substitutes the active slot. This means **any** POID can become multi-slot by flipping one manifest flag, with no code changes.

### 6.5 Vault storage engine

The vault **MUST** be implemented as a CRDT (RECOMMENDED: Automerge), not as a last-write-wins blob.

> **Rationale:** This is a load-bearing decision. Without CRDT semantics from day one, offline editing on two devices loses data, and adding synchronisation later requires rewriting the storage engine. Automerge has both a Rust core (for the native reader) and a WASM binding (for the web reader), so a single engine serves every platform.

---

## 7. Runtime API and the broker

Applications interact with the outside world **only** through the `poid.*` API, injected into the sandboxed context. Every call is a message to the **broker** in the reader core, which:

1. checks the manifest declaration,
2. checks the user's granted permissions,
3. performs the operation,
4. returns only the result.

### 7.1 Credential isolation — normative

> **The application MUST NEVER receive credentials.**
> API keys, passwords, OAuth tokens, and database connection strings are held by the reader (in the OS keychain) and are **never** exposed to the sandboxed context — not in memory, not in a response, not in an error message.
>
> An application calls `poid.db.sql("SELECT ...")`. The **broker** executes it. A malicious POID has nothing to steal.

This property applies uniformly to databases, sync, AI model keys, and MCP servers.

See `RUNTIME-API.md` for the complete API surface.

---

## 8. Data Engine

Key-value storage is insufficient for real applications. Conformant readers **MUST** provide:

| API | Backing | Purpose |
|---|---|---|
| `poid.db.kv` | vault / `data/store.json` | simple key-value |
| `poid.db.sql` | wa-sqlite (WASM) or PGlite | relational queries |
| `poid.db.docs` | document store over SQLite | collections, Mongo-like queries |
| `poid.db.remote` | external, via a Connection | user-configured backend |

Applications target the **POID Data Engine**, not MongoDB or PostgreSQL directly. Porting an existing application means rewriting its data layer against this API. This is a **port**, not a one-click conversion.

`migrations/` **MAY** contain ordered migration scripts, applied by the reader when an application is updated in place and the schema version has advanced (§12).

---

## 9. Security

### 9.1 Permission model

`permissions` in the manifest is a **request**, not a grant. On first open, the reader **MUST** show a **preview mode** presenting:

- application name, version, author, signature status
- the exact permissions requested, in plain language
- optionally, the source code

Execution **MUST NOT** begin until the user explicitly consents. Consent **MUST** be recorded per `app.id` + content hash. Any change to the content hash **MUST** re-trigger consent.

Default posture: **deny**. An app with `"network": []` **cannot** make a network request — this is enforced by CSP, not by convention.

### 9.2 `protected` storage

When `storage.protected` is `true`, `data/` **MUST** be encrypted with **AES-256-GCM**, with the key derived from a user passphrase via **Argon2id**.

> This is real encryption, not a UI lock. A UI-only lock is trivially defeated by unzipping the file.
>
> Readers **SHOULD** advise users that *"Send without data"* is strictly safer than *"Send encrypted"* — data that is absent cannot leak.

### 9.3 Signatures

`signature/` **MAY** contain an **Ed25519** signature attesting the application content. Readers **SHOULD** display a *verified publisher* indicator for validly signed POIDs. Enterprise policy (§13) **MAY** require a valid signature from a trusted key before execution.

#### 9.3.1 Signature file (normative)

A signed POID contains exactly one file, `signature/signature.json`:

```json
{
  "version": 1,
  "algo": "ed25519",
  "public_key": "…32 bytes, lowercase hex…",
  "signature": "…64 bytes, lowercase hex…"
}
```

#### 9.3.2 Signed payload (normative)

The signature is computed over the UTF-8 bytes of a canonical JSON object built
from the manifest with the following fields, in this order, omitting absent
ones — compact serialization (no whitespace), preserved unknown fields inside
the included blocks in lexicographic key order:

`poid`, `type`, `app`, `runtime`, `entry`, `permissions`, `shared_scope`,
`data_ref`, `integrity_app` (the value of `integrity.app`), `integrity_deps`
(the value of `integrity.deps`).

The payload deliberately **excludes**:

- `instance` — assigned per copy by the reader on first open (§6.3);
- `storage` — the storage location is the *user's* choice, and readers MUST
  support mode conversion (§6.1); it is not part of the publisher's attestation;
- `draft` — authoring state;
- unknown fields at the manifest's top level — they may be added by
  intermediate tooling.

Because the payload includes the `integrity` digests (§3.3), a valid signature
transitively attests the full content of `app/` and `deps/`. Any change to the
application content therefore requires re-signing; assigning an instance
identity, saving user data, or converting the storage mode does not.

### 9.4 Resource governance

Readers **MUST** enforce:
- a storage quota per POID (default: 64 MB; configurable)
- a wall-clock execution watchdog, with a user-visible **"Stop this application"** control
- CPU and memory limits, where the platform permits

---

## 10. Workspaces

`type: "workspace"` contains nested POIDs under `apps/`. They **MAY** be embedded (self-contained, shareable) or referenced by path (lightweight, local).

The workspace window presents a sidebar, tabbed switching, and an optional tiled view. **Each nested POID runs in its own sandbox.**

By default, nested POIDs are **isolated from one another** — as if they were separate files.

A workspace **MAY** declare `shared_scope: ["projects"]`, granting nested apps access to a shared namespace. The reader **MUST** display a persistent indicator that these applications share data, and **MUST** obtain explicit user consent.

---

## 11. Data containers (`type: "data"`)

Used for offline collection workflows:

1. Author creates `survey.poid` (`type: app`) and emails it.
2. Respondent opens it (Studio or web reader), fills it in, clicks *Submit*.
3. The reader produces `response.poid` (`type: data`, a few KB, no code) and the respondent returns it.
4. Author drags 200 response files onto the original survey → the reader imports and aggregates them.

Zero servers, zero cloud, zero GDPR exposure. This is a first-class workflow, not a fallback.

---

## 12. Application updates

Readers **MUST** support **"Update program, keep data"**:

1. Replace `app/`, `deps/`, and the relevant `runtime` fields from a newer POID with the same `app.id`.
2. Preserve `data/` (or the vault entry) and `instance.id`.
3. If the data schema version has advanced, apply `migrations/` in order.
4. Re-request consent if permissions have widened.

Without this, POIDs are disposable and nobody will build anything serious on the format.

---

## 13. Enterprise policy (informative)

Readers **MAY** support a managed policy layer (GPO / MDM / configuration file) that the user cannot override:

| Policy | Effect |
|---|---|
| `storage.mode = "vault"` (forced) | Data never enters a file |
| `export.embedded = false` | *"Send with data"* is removed from the UI |
| `network.allowlist = [...]` | POIDs may only reach approved origins |
| `resolver = "offline"` | Studio never contacts a package registry |
| `vault.backend = <url>` | Memory lives on the organisation's server |
| `signing.required = true` | Only POIDs signed by a trusted key will run |
| `audit = true` | Open and export events are logged |

---

## 14. Conformance

An implementation is **conformant** if it:

1. Passes the POID Conformance Suite (`spec/conformance/`).
2. Rejects every file in `spec/conformance/invalid/` with the specified error code (registry: `spec/errors.md`).
3. Correctly opens every file in `spec/conformance/valid/`.
4. Enforces §5.2 (isolation), §7.1 (credential isolation), and §9.1 (consent).

A reader that does not enforce §5.2, §7.1, and §9.1 **MUST NOT** be described as a POID reader.

---

## 15. Registration

- **Media type:** `application/vnd.poid+zip` — to be registered with IANA in the vendor tree.
- **Magic bytes:** `PK\x03\x04` followed by a `mimetype` entry containing `application/vnd.poid+zip`.
- **UTI (macOS):** `dev.poid.document`, conforming to `public.zip-archive` and `public.data`.
