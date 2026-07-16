# @poid/web

The Web Reader — a static, 100% client-side SPA for recipients who have not
installed Studio. Drag a `.poid` onto the page and it runs. The server is a
CDN serving static files; it never sees a POID.

## How it works

- **Validation** is `poid-core` compiled to WASM (`crates/poid-wasm`) — the
  same Rust code as the desktop reader, not a TypeScript reimplementation.
  Rejections show the normative `POID-xxx` code and a human explanation.
- **Consent, sandbox, broker, watchdog** are `@poid/ui` + `@poid/host` — the
  same modules the desktop reader mounts. Consent first, execution second.
- **Storage** is `embedded` mode backed by IndexedDB (`IndexedDbEngine`),
  keyed by `instance.id`, with an explicit **Download updated file** button
  that packs the current state back into a conformant container.
- **PWA**: installable, registers as a `.poid` file handler (Chromium), and a
  service worker precaches the shell — fully functional offline after the
  first load.

## Honest limitations (shown in the UI, not hidden)

- No double-click opening from the desktop without installing the PWA, and no
  write-back to the original file — saving is a download.
- No OS keychain: no Connections, no AI keys. `poid.ai` / remote backends are
  Studio features.
- No extra engines yet: `web+python` (Pyodide) and friends get an honest
  "engine not available" screen.
- Multi-file applications: parity with the desktop reader of M04 — the entry
  document runs sandboxed; subresource serving behind a synthetic origin is a
  later milestone ([#5](https://github.com/POIDIT/POID/issues/5)).

## Build and run

```
pnpm --filter @poid/web build:wasm    # needs Rust + wasm32-unknown-unknown target
pnpm --filter @poid/web build:site    # bundles into site/ (static files only)
node packages/poid-web/e2e/server.mjs # builds + serves on :5179
```

## Test

```
pnpm --filter @poid/web test          # vitest: engine, facts, error registry
pnpm --filter @poid/web e2e           # Playwright: conformance fixtures in a real
                                      # Chromium, zero-network proof, offline proof
```

## Deploy (Cloudflare Pages)

The site is plain static output. Point Pages at this repo with:

- **Build command:** `pnpm install --frozen-lockfile && pnpm --filter @poid/web build:wasm && pnpm --filter @poid/web build:site`
- **Build output directory:** `packages/poid-web/site`
- **Environment:** Rust via `rustup` is preinstalled on Pages build images;
  add `rustup target add wasm32-unknown-unknown` before the build command if
  the image lacks the target.

No Functions, no KV, no server code — a strict CDN deployment.
