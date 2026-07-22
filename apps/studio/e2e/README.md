# The desktop test tier

These tests drive **POID Studio itself** — the built binary, its windows and
its Rust backend — rather than a page in a browser.

Everything else in this repository stops short of that. `vitest` covers logic,
and the Playwright suites in `@poid/host` and `@poid/web` drive the *web*
reader in Chromium. Until this tier existed, the desktop product was verified
only by whatever the maintainer remembered to click, which `KNOWN-GAPS.md`
named the most important gap at the end of M11.

## Running them

```bash
pnpm --filter @poid/studio build:ui
cargo build --manifest-path apps/studio/src-tauri/Cargo.toml --release
pnpm --filter @poid/studio e2e
```

The build steps are separate on purpose: these tests never build the
application. A test that silently triggers a fifteen-minute release link is a
test nobody runs twice. A `debug` build is used if no `release` one exists, and
`POID_STUDIO_BIN` overrides both.

**Close every Studio and Reader window first.** Studio holds a single-instance
lock, so a launch while one is running is forwarded into that process; the
harness detects this and stops with an explanation rather than driving the
wrong application.

## What it can and cannot do

| | |
|---|---|
| Platform | **Windows only.** Tauri's webview there is WebView2, which is Chromium and speaks the Chrome DevTools Protocol. macOS and Linux use WebKit, which does not. |
| Parallelism | One worker. See the single-instance lock above. |
| Your data | Untouched. Each launch gets a temporary `APPDATA`, so the vault and the connection registry start empty and are deleted afterwards. |
| Your documents | `launchStudio({ open })` opens a **copy**. Opening a POID rewrites it — the reader stamps `instance.id` into the manifest (SPEC §3.2, §6.3) — so pointing a test straight at a conformance fixture would dirty a file the repository generates deterministically. |

## One side effect worth knowing

Studio repairs its own `.poid` file association at startup (`association.rs`),
pointing it at the running executable. That is deliberate — it is how a dev
build behaves like an installed one — but it means running this suite from a
normal Windows session re-points `.poid` files at the build under
`target/`. Launching the installed Studio once puts it back.

## Adding a test

`studio-app.ts` is the whole API:

```ts
app = await launchStudio({ open: fixture("valid/minimal-html.poid") });
const reader = await app.reader();
await reader.locator(".poid-consent__run").click();
const frame = await app.appFrame(reader);
```

`app.hub()` and `app.reader()` return Playwright `Page` objects, so selectors
and auto-waiting work as usual. `app.appFrame(reader)` returns the sandboxed
application, which is a cross-origin `poid://` document and therefore an
out-of-process iframe — a *frame* of the reader page, never a page of its own.
`app.stderr()` returns whatever the application printed, which is where a Rust
side failure will be waiting for you. Always `await app.close()`.
