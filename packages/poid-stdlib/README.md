# @poid/stdlib

The Standard Library (ARCHITECTURE §5.2, Tier 1): a **closed, curated,
versioned** set of pre-bundled ESM libraries shipped with Studio. It covers
what AI chatbots actually emit — bare imports like `react`, `chart.js/auto`
or `lodash-es` resolve locally, offline, deterministically.

This is **not** a package manager and must never become one. The catalog
(`src/catalog.json`) is the whole universe: pinned npm versions, pinned
tarball integrity, a pinned dependency closure per package, and the sha256
of every built bundle. Anything outside it is answered with a clear miss —
the Resolver (Tier 2) may then offer a consented download at authoring time.

## Layout

- `src/catalog.json` — the committed source of truth (checksums included).
- `src/index.ts` — the map: `resolveStdlib(bareImports)` → bundle selection
  (externals followed transitively), `exclusionReason()` for curated gaps.
- `scripts/build-stdlib.mjs` — dev-time builder: downloads the pinned
  tarballs (the one place in the repo that talks to npm), verifies
  integrity, bundles each specifier with the pinned esbuild, and verifies
  every sha256. `--write` re-records pins; the diff is reviewed like code.
- `lib/` — the built bundles (~7 MB). **Not committed**; CI rebuilds and
  asserts the committed checksums, which is the reproducibility gate.

## Build

```
pnpm --filter @poid/stdlib build:lib          # verify against the catalog
pnpm --filter @poid/stdlib build:lib -- --write  # re-pin after a curation change
```

## Notes

- Bundles are **unminified** so tree-shaking annotations survive into the
  app-level build (a `lucide-react` app ships the icons it uses, not all of
  them); the app build minifies.
- CommonJS packages get a curated named-exports shim, and `require()` calls
  to externalized peers are rewritten to static imports — both documented in
  the build script.
- Licenses of redistributed libraries stay in the bundles
  (`--legal-comments=inline`).
- Curated exclusions (with reasons shown to authors): `svelte` (compiler),
  `tailwindcss` (Converter concern), `sql.js` (wasm shim) — see
  `catalog.json`.
