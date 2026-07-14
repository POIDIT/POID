# POID — System Architecture

> Companion to `SPEC.md`. The spec defines the **file format**; this document defines the **software that implements it**.

---

## 1. The one-executable rule

There is exactly **one** executable in the entire POID system: **POID Studio**.

| | File | Program |
|---|---|---|
| Word | `report.docx` | `WINWORD.EXE` |
| PDF | `contract.pdf` | `Acrobat.exe` |
| **POID** | **`kanban.poid`** | **`POID Studio`** |

A `.poid` never becomes an `.exe`, never generates one, and never contains one.

---

## 2. Two window classes, one process

POID Studio launches in one of two modes.

### Reader mode — `poid-studio --open <path>`
Triggered by double-clicking a `.poid`. Opens **only a document window**: title bar, the mini-program, and a thin toolbar (Save, Share, slot switcher, storage badge). **No hub, no IDE, no panels.**

### Studio mode — launched from the app menu
Opens the **hub**: converter, editor, AI, library, connections, settings.

These are two window types of the same application, exactly as Word shows a document when you open a `.docx` and a start screen when you launch it from the Start menu.

### Single-instance lock

1. First launch acquires a named mutex / local socket.
2. Subsequent launches detect the lock, **hand the file path to the running process, and exit**.
3. The running process opens a **new window**.

Result: N independent Reader windows on the desktop, one process, one shared vault, one set of loaded engines. Each window has its own taskbar entry. Just like Excel.

---

## 3. Layers

```
┌────────────────────────────────────────────────────────────┐
│  POID Studio Core   (Rust / Tauri)                         │
│                                                            │
│  • Container      read/write/validate .poid                │
│  • Vault          Automerge CRDT, slots, instance identity │
│  • Data Engine    kv / sql / docs / remote                 │
│  • Connections    OS keychain, credential broker           │
│  • Toolchain      esbuild, Standard Library, Resolver      │
│  • Engines        Pyodide, wa-sqlite … (verified, cached)  │
│  • Policy         enterprise configuration                 │
│  • BROKER         ← every privileged operation passes here │
└────────────────────────────────────────────────────────────┘
        │ IPC                              │ IPC
        ▼                                  ▼
┌───────────────────────┐      ┌──────────────────────────────┐
│  Studio Window        │      │  Reader Window × N           │
│  (hub, IDE, AI)       │      │  ┌────────────────────────┐  │
│                       │      │  │ <iframe sandbox> + CSP │  │
│                       │      │  │   the mini-program     │  │
│                       │      │  │   window.poid = SDK    │  │
│                       │      │  └────────────────────────┘  │
└───────────────────────┘      └──────────────────────────────┘
                                          │ postMessage only
                                          ▼
                                    (broker in Core)
```

**The mini-program never talks to the operating system.** It calls `poid.db.set(...)`; that becomes a `postMessage` to the Reader window, then IPC to Core, where the broker checks the manifest and the user's grants before touching anything. One narrow gate, one place to audit.

If you compromise this boundary, the whole security model collapses. This is the code you write most carefully.

---

## 4. Repository layout (monorepo)

```
poid/
├── crates/                       # Rust
│   ├── poid-core/                # container, manifest, validation, integrity
│   ├── poid-vault/               # Automerge storage, slots, instance identity
│   ├── poid-broker/              # permission enforcement, Data Engine, Connections
│   └── poid-cli/                 # `poid` command-line tool
│
├── packages/                     # TypeScript
│   ├── poid-sdk/                 # runs INSIDE the sandbox → window.poid
│   ├── poid-host/                # host side of the postMessage bridge
│   ├── poid-toolchain/           # esbuild-wasm build + Standard Library + Resolver
│   ├── poid-ui/                  # shared UI components (Reader chrome, consent dialog)
│   └── poid-web/                 # Web Reader — static SPA, 100% client-side
│
├── apps/
│   ├── studio/                   # Tauri v2 — desktop + iOS + Android
│   ├── cloud/                    # Cloudflare Workers — POID Cloud
│   └── mcp-server/               # MCP server: lets AI agents emit .poid files
│
├── spec/
│   ├── SPEC.md
│   ├── schema/manifest.schema.json
│   └── conformance/{valid,invalid}/
│
└── examples/                     # notepad, kanban, survey, python-chart, cad
```

**Why Rust core + TS runtime:** the Core must run identically on five platforms and inside the CLI. Rust + Tauri v2 gives desktop *and* mobile from one codebase. The sandboxed runtime must be JS because that is what browser engines execute.

---

## 5. The Toolchain (this is where "no terminal" is earned)

The user must never see a terminal. That means Studio must do, silently, what a developer normally does with `npm install && npm run build`.

### 5.1 Build

- **In Studio / Web Reader:** `esbuild-wasm`, running inside the WebView. **No Node.js anywhere.**
- **In the CLI:** the native `esbuild` binary, shipped as a sidecar.
- The **exact esbuild version MUST be pinned** and recorded in `runtime.toolchain`, so builds are reproducible.

Building a React app: ~200 ms. The user sees a progress bar, never a console.

### 5.2 Dependency resolution — three tiers

**Tier 1 — Standard Library (offline, instant, ~95% of cases)**
Studio ships a curated, pre-bundled set of the most common libraries (~30–50 MB):

> react, react-dom, vue, svelte, tailwind, recharts, chart.js, d3, three.js, lucide, lodash, date-fns, zod, papaparse, sql.js, marked, framer-motion, xlsx …

This is **not** a local npm. It is a closed, versioned, curated list. It covers virtually everything AI chatbots generate, because they are limited to a similar set. Zero network, zero waiting, fully deterministic.

**Tier 2 — Resolver (author's machine, with consent, at authoring time only)**
> *"This app uses `tone` (audio, 180 KB), which I don't have locally. I can download it from npm and **store it permanently inside this POID**. [Download] [Cancel]"*

Studio downloads it, verifies the checksum, and bundles it **into the file**. The recipient downloads nothing.

**Tier 3 — Offline mode**
Enterprise / school / archive: Tier 2 disabled. Standard Library only. Clear error if something is missing.

### 5.3 The network rule — normative

> The network is touched **only** by POID Studio, **only** in authoring mode, **only** with user consent, **only** to place a dependency inside a file.
> Opening a POID **never** generates network traffic.
> A running mini-program **physically cannot** make a request (`connect-src 'none'`).

---

## 6. POID Cloud

### 6.1 Why the servers won't fall over

`app.poid.dev` is a **static site on a CDN**. Everything happens in the user's browser:

| Operation | Where it runs |
|---|---|
| unzip the container | JSZip, in the browser |
| build | esbuild-wasm, in the browser |
| Python | Pyodide, in the browser |
| SQL | wa-sqlite / PGlite, in the browser |
| sandbox | `<iframe sandbox>` + CSP, in the browser |

**No server ever executes user code.** POID Cloud cannot be brought down by compute load, because there is no compute. Only *data* scales — and data is metered and billable.

### 6.2 Service catalogue

POID Cloud is not "Studio in the cloud". It is a catalogue of **backend services a mini-program needs but a file cannot provide**. It plugs into **Connections** as one implementation among many — the user can always point at their own Supabase instead.

| Service | Purpose | Replaces |
|---|---|---|
| **Sync** | POID memory across PC / phone / web | Dropbox, manual copying |
| **Publish** | a POID at a URL, opens without installing anything | Vercel, Netlify |
| **Collect** | gather responses (surveys, forms, submissions) | Google Forms, Typeform |
| **DB** | SQL / document store bound to a POID | Supabase, Mongo Atlas |
| **Files** | blob storage | S3 |
| **Auth** | sign-in for a published POID | Auth0, Clerk |
| **AI Gateway** | one key → many models, quotas, cost tracking | OpenRouter |
| **Registry** | publish and discover POIDs, signed publishers | — |
| **Teams / Policy** | enterprise policy, audit, forced signing | Intune / GPO |

### 6.3 Business model

```
Spec + Studio + Reader + CLI   →  open source, free, forever
BYO backend (Supabase, VPS)    →  free, supported, documented
POID Cloud Free                →  5 active POIDs with services, quotas
POID Cloud Pro                 →  subscription, unlimited POIDs, more data
POID Cloud Team / Enterprise   →  policy, SSO, audit, self-host, support
```

**Nobody is locked in.** If POID Cloud disappears, every POID keeps working — memory can be switched back to `embedded` with one click. Say this loudly; it is rare and people notice.

---

## 7. Platform matrix

| Platform | Framework | File association | Notes |
|---|---|---|---|
| Windows | Tauri v2 (WebView2) | registry `HKCR\.poid` + ProgID | **Code signing mandatory** (SmartScreen) |
| macOS | Tauri v2 (WKWebView) | Exported UTI in `Info.plist` | Notarisation required |
| Linux | Tauri v2 (WebKitGTK) | `.desktop` + `shared-mime-info` | Flatpak + AppImage + .deb |
| iOS | Tauri v2 mobile | Document Types + UTI | Reader + AI + sync. **No terminal, no full IDE** (App Store rule 2.5.2) |
| Android | Tauri v2 mobile | `intent-filter` | Same scope as iOS |
| Web | static SPA | drag & drop / File Handling API | Fallback for recipients without Studio. **Not optional — adoption depends on it.** |

### App Store risk (rule 2.5.2)

Apple forbids downloading and executing code. **Exception: code executed by WebKit.** POID runs JS/WASM in WKWebView, which falls inside the exception (precedents: Scriptable, a-Shell, Playgrounds).

Mitigation:
- Never describe the app as "runs arbitrary code". Describe it as **"opens interactive POID documents"** — which is true, and is how Word is described.
- Ship no terminal and no full IDE on iOS.
- The Web Reader is the fallback if review goes badly.

---

## 8. The MCP server (`apps/mcp-server`) — the adoption lever

Every AI chatbot today generates applications that **have nowhere to live**. A Claude artifact lives in the chat. A GPT canvas lives in the chat. Close the tab and it's gone.

POID is the missing output format for the age of AI-generated applications.

The MCP server exposes tools such as `poid_create`, `poid_validate`, `poid_inspect`. Once installed, Claude / ChatGPT / Cursor / Kilo Code can **save their output as a `.poid` file directly**.

> User: *"Build me a kanban board and save it as a POID."*
> → `kanban.poid` appears on the desktop. Double-click. It runs.

This turns POID into the **"Save" button for the entire AI industry**. It is the single highest-leverage component in the project.
