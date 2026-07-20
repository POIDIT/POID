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
- Multi-file applications run (M09, [#5](https://github.com/POIDIT/POID/issues/5)):
  the synthetic origin (SPEC §5.2.1) rewrites the entry's `src`/`href`
  references to `data:` URLs of the container files, so an external
  `<script src="main.js">` executes. References buried in CSS `url()` or
  resolved by runtime dynamic imports are out of scope — every POID is bundled
  at authoring time, which this serves losslessly.

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

## Deploy (Cloudflare)

The site is plain static output; either Cloudflare surface works. No
Functions, no KV, no server code — a strict CDN deployment.

**Workers (Import a repository)** — uses `wrangler.jsonc` in this package
(an assets-only Worker, no script). In the Worker's *Settings → Build*:

- **Root directory:** `/` (the pnpm workspace install runs at the repo root)
- **Build command:**
  `rustup target add wasm32-unknown-unknown && pnpm install --frozen-lockfile && pnpm --filter @poid/web build:wasm && pnpm --filter @poid/web build:site`
- **Deploy command:** `npx wrangler deploy --config packages/poid-web/wrangler.jsonc`

**Pages (Connect to Git)** — no wrangler file needed:

- **Build command:** same as above
- **Build output directory:** `packages/poid-web/site`
