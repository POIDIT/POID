/**
 * `@poid/stdlib` — the Standard Library catalog (ARCHITECTURE §5.2, Tier 1).
 *
 * A closed, curated, versioned list of pre-bundled ESM libraries. This module
 * is the *map*: which bare specifiers exist, which file serves each one, and
 * what its checksum must be. The bundles themselves are built by
 * `scripts/build-stdlib.mjs` from pinned npm inputs and shipped with Studio;
 * they are not committed to the repository — CI rebuilds them and asserts
 * the checksums recorded here.
 *
 * This is not a package manager and must never become one: requests to
 * resolve anything outside the catalog are answered with a clear "not in the
 * Standard Library", never with a fetch.
 */

import catalogJson from "./catalog.json" with { type: "json" };

/** One resolvable bare specifier. */
export interface StdlibSpecifier {
  /** The npm package the bundle was built from. */
  pkg: string;
  /** Pinned package version. */
  version: string;
  /** Bundle path relative to the library root, e.g. `react/jsx-runtime.js`. */
  file: string;
  /** SHA-256 of the built bundle (lowercase hex). */
  sha256: string;
  /** Bare imports the bundle leaves external (resolved by further aliases). */
  externals: string[];
}

interface CatalogSpecifier {
  entry: string;
  externals: string[];
  sha256?: string;
  namedExports?: string[];
}

interface CatalogPackage {
  version: string;
  integrity: string;
  specifiers: Record<string, CatalogSpecifier>;
}

interface Catalog {
  esbuild: string;
  packages: Record<string, CatalogPackage>;
  excluded: Record<string, string>;
}

const catalog = catalogJson as unknown as Catalog;

/** Bundle path (relative to the library root) for a bare specifier. */
export function bundleRelPath(specifier: string): string {
  return `${specifier}.js`;
}

/** The full specifier map, keyed by bare specifier. */
export function stdlibSpecifiers(): Map<string, StdlibSpecifier> {
  const out = new Map<string, StdlibSpecifier>();
  for (const [pkg, entry] of Object.entries(catalog.packages)) {
    for (const [specifier, spec] of Object.entries(entry.specifiers)) {
      out.set(specifier, {
        pkg,
        version: entry.version,
        file: bundleRelPath(specifier),
        sha256: spec.sha256 ?? "",
        externals: spec.externals,
      });
    }
  }
  return out;
}

/** The result of resolving a set of bare imports against the catalog. */
export interface StdlibResolution {
  /** Specifier → bundle path (relative to the library root), including the
   * transitive externals of every selected bundle. */
  selection: Map<string, StdlibSpecifier>;
  /** Bare imports the catalog cannot serve, sorted. */
  missing: string[];
}

/**
 * Resolves bare imports to catalog bundles, following externals transitively
 * (selecting `react-dom` pulls in `react`). Never fetches anything.
 */
export function resolveStdlib(bareImports: Iterable<string>): StdlibResolution {
  const specs = stdlibSpecifiers();
  const selection = new Map<string, StdlibSpecifier>();
  const missing = new Set<string>();
  const queue = [...bareImports];
  while (queue.length > 0) {
    const name = queue.pop();
    if (name === undefined || selection.has(name)) continue;
    const spec = specs.get(name);
    if (!spec) {
      missing.add(name);
      continue;
    }
    selection.set(name, spec);
    queue.push(...spec.externals);
  }
  return { selection, missing: [...missing].sort() };
}

/** Why a known-but-excluded library is not available, if it is one. */
export function exclusionReason(specifier: string): string | undefined {
  const pkg = specifier.split("/")[0] ?? specifier;
  return catalog.excluded[pkg];
}

/** The exact esbuild version the library was built with (must match the
 * build contract). */
export const STDLIB_ESBUILD = catalog.esbuild;
