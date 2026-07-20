# @poid/studio — POID Studio (desktop)

The one executable of the POID system (Tauri v2). Double-clicking a `.poid`
opens a **Reader window** — a document window with the running application
and nothing else. Launching the app bare opens the **Studio hub**. The hub
never appears because a file was opened; that separation is a UX rule, not a
styling choice.

## Layout

- `src-tauri/` — the Rust side (crate `poid-studio`, **outside** the Cargo
  workspace; it only builds where a system webview exists). Owns container
  opening (`poid-core`), window creation, the single-instance lock, and the
  per-user file association repair on Windows.
- `src/` — the window frontends: `reader-main.ts` (Reader), `hub-main.ts`
  (hub), plus the IPC document contract (`document-dto.ts`) and the routing
  (`desktop-flow.ts`) that mirrors the Web Reader's `openBytes`.
- `static/` + `scripts/build-ui.mjs` — the two window pages, bundled with
  esbuild into `dist-ui/` (Tauri's `frontendDist`). The `@poid/sdk` bootstrap
  is injected as `__SDK_SOURCE__`, exactly like the Web Reader's site build.
- `brand/` — source logo art. The **app icon** (Studio logo) and the
  **document icon** (format logo) are deliberately different: a user must be
  able to tell "this is a document" from "this is a program" at a glance.
  Generated sets live in `src-tauri/icons/` (committed).

## Running it

```
pnpm install && pnpm -r build          # workspace packages once
pnpm --filter @poid/studio tauri dev   # or: tauri build
```

`tauri build` produces MSI + NSIS on Windows, `.dmg` on macOS, and
`.deb`/`.rpm`/AppImage on Linux (the Studio CI job builds all three).
Local builds are unsigned; code signing is tracked separately.

## How a document reaches a window (and why)

1. Launch argv (or a forwarded second-instance argv, or a macOS `Opened`
   event) is parsed by `cli::open_request`.
2. The Rust side opens and validates the file with `poid-core`, then stores
   the outcome **keyed by the new window's label** (`state::Documents`).
   A rejected container still gets a window — the honest explanation panel;
   a double-click must never silently do nothing.
3. The window's TypeScript calls `reader_bootstrap`, which looks up the
   document by **the calling window's label**. There is no variant taking a
   label or path parameter: scope is derived from the window (security
   rule 3), so one window can never read another window's document.
4. `desktop-flow.routeDocument` interprets the manifest with the same
   `extractFacts` as the Web Reader and mounts via `@poid/host`'s
   `mountReader` — the same consent, sandbox, broker and watchdog on both
   platforms.

Each Reader window hosts its own sandboxed iframe; a crash or hang inside
one POID takes down one window, not the process (the watchdog marks the
chrome unresponsive and Stop always works).

## Security notes specific to the desktop shell

- **Host pages ship no CSP** (`app.security.csp: null`), mirroring the Web
  Reader. This is deliberate: the application iframe is a `blob:` document,
  and blob documents **inherit the creator's CSP** — any host `script-src`
  would also govern (and break) the sandbox's injected runtime. The
  sandbox's own protection is unaffected: its document carries its own CSP
  (`connect-src 'none'`, …) and the iframe runs `sandbox="allow-scripts"`
  without `allow-same-origin`. A strict host CSP becomes possible again once
  applications are served from a real synthetic origin (issue #5).
- **WebView2 injects Tauri's IPC object into every frame**, including the
  sandboxed one, and it is not deletable (non-configurable). Verified
  empirically: an `invoke` from the sandbox's opaque origin is dropped by
  Tauri's origin gate — it never reaches Rust, opens no window, returns
  nothing (see the M07 PR for the probe transcript). The application's only
  working channel is the audited `postMessage` bridge. Hardening this from
  "dead object" to "no object" (Tauri isolation pattern) is a tracked
  follow-up.
- The Windows association repair writes **HKCU only** (no admin), is
  idempotent, and re-points icon + open command at the current binary on
  every launch — so dev builds behave like installed ones and a broken
  association heals itself.

## Deliberately not in M07

- **Engines beyond plain `web`** (Pyodide on desktop): a `web+python` POID
  gets an honest "engine not available yet" notice, like the Web Reader
  before M06 wiring. Follow-up milestone.
- **Writing `instance.id` back into the file** on first open (SPEC §6.3):
  assigned in memory for the storage scope, persisted with the vault/save
  milestone.
- **Auto-update**: needs a release feed, CI secrets and the signing
  decision; wiring it half-dead would only pretend. Tracked as an issue.
- **Synthetic origin for multi-file apps**: issue #5, the next milestone —
  M07 runs the M06 single-file/inlined profile.
