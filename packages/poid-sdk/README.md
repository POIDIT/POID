# @poid/sdk

The API injected into the sandboxed context as `window.poid` (RUNTIME-API.md).
Zero runtime dependencies; under 10 KB minified (enforced by a size test).

- Every method is a `postMessage` with a request id, returning a Promise, and
  rejects with a typed `PoidError` carrying the RUNTIME-API §9 codes.
- `poid.net`, `poid.ai`, `poid.mcp` (and `db.sql`, `ui.print`, `ui.clipboard`)
  are **absent** — not throwing stubs — unless the capability was granted, so
  an application can feature-detect what it may do.

The host injects the `./bootstrap` entry (bundled to a self-executing IIFE) as
the first script in the application document, so `window.poid` exists before
any application code runs.

```
pnpm --filter @poid/sdk build
pnpm --filter @poid/sdk test
```
