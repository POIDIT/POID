/**
 * The Resolver — Tier 2 of dependency resolution (ARCHITECTURE §5.2) — and
 * the offline policy that is Tier 3.
 *
 * The shape of this API *is* the consent flow: `planDownload` only reads
 * metadata and returns a description the UI shows to the author; nothing is
 * downloaded until the caller passes that plan to `downloadAndBundle`. There
 * is no single call that resolves-and-fetches, so an unconsented download is
 * not expressible. Everything runs at authoring time on the author's
 * machine; the result is stored **inside the POID** and recorded in
 * `runtime.bundled_deps` — the recipient downloads nothing, ever
 * (ARCHITECTURE §5.3).
 *
 * Tier 3: `{ offline: true }` disables the Resolver entirely with a clear
 * error — the mode institutions run.
 */

import { type BundleInput, bundle, type EsbuildApi } from "./engine.js";
import { gunzip, untar } from "./tar.js";

/** The Resolver policy. Offline = Standard Library only (Tier 3). */
export interface ResolverPolicy {
  offline: boolean;
}

/** What would be downloaded, shown to the author before any download. */
export interface DownloadPlan {
  /** The bare specifier the project imports. */
  specifier: string;
  /** npm package name (the specifier's package part). */
  pkg: string;
  /** The exact version that would be stored (latest stable at plan time). */
  version: string;
  /** Unpacked size in bytes, when the registry reports one. */
  sizeBytes?: number;
  /** Tarball URL. */
  tarball: string;
  /** Registry integrity (`sha512-…`) the download must match. */
  integrity: string;
}

/** A dependency bundled into the project after a consented download. */
export interface ResolvedDependency {
  /** Project file path of the bundled ESM, e.g. `vendor/tone.js`. */
  file: string;
  /** The bundle content. */
  content: Uint8Array;
  /** The `name@version` line for `runtime.bundled_deps`. */
  record: string;
  /** The alias entry to add to the build. */
  specifier: string;
}

/** Raised when Tier 3 policy forbids the Resolver. */
export class OfflinePolicyError extends Error {
  constructor(specifiers: string[]) {
    super(
      `offline mode: cannot download ${specifiers.map((s) => `\`${s}\``).join(", ")}. ` +
        "This build is restricted to the Standard Library; add the dependency to the " +
        "project as source files, or build on a machine where downloads are allowed.",
    );
    this.name = "OfflinePolicyError";
  }
}

/** Raised when a download does not match the registry integrity. */
export class IntegrityError extends Error {
  constructor(pkg: string) {
    super(`the downloaded tarball for \`${pkg}\` does not match its registry checksum`);
    this.name = "IntegrityError";
  }
}

type FetchLike = (url: string) => Promise<{
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
  arrayBuffer(): Promise<ArrayBuffer>;
}>;

function packageOf(specifier: string): string {
  const parts = specifier.split("/");
  return specifier.startsWith("@") ? parts.slice(0, 2).join("/") : (parts[0] ?? specifier);
}

/**
 * Reads registry metadata for a missing bare import and describes what a
 * download would store. Never downloads the tarball. Throws
 * {@link OfflinePolicyError} under Tier 3.
 */
export async function planDownload(
  specifier: string,
  policy: ResolverPolicy,
  fetchImpl: FetchLike,
): Promise<DownloadPlan> {
  if (policy.offline) throw new OfflinePolicyError([specifier]);
  const pkg = packageOf(specifier);
  const res = await fetchImpl(`https://registry.npmjs.org/${pkg}/latest`);
  if (!res.ok) throw new Error(`npm registry: HTTP ${res.status} for \`${pkg}\``);
  const meta = (await res.json()) as {
    version?: string;
    dist?: { tarball?: string; integrity?: string; unpackedSize?: number };
  };
  if (!meta.version || !meta.dist?.tarball || !meta.dist.integrity) {
    throw new Error(`npm registry returned no usable release for \`${pkg}\``);
  }
  const plan: DownloadPlan = {
    specifier,
    pkg,
    version: meta.version,
    tarball: meta.dist.tarball,
    integrity: meta.dist.integrity,
  };
  if (meta.dist.unpackedSize !== undefined) plan.sizeBytes = meta.dist.unpackedSize;
  return plan;
}

async function sha512base64(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-512", bytes.slice().buffer);
  let binary = "";
  for (const b of new Uint8Array(digest)) binary += String.fromCharCode(b);
  return `sha512-${btoa(binary)}`;
}

/** Resolves the package entry file from its manifest (module-first). */
function entryOf(manifest: { exports?: unknown; module?: string; main?: string }): string {
  if (typeof manifest.exports === "object" && manifest.exports !== null) {
    const dot = (manifest.exports as Record<string, unknown>)["."];
    const pick = (v: unknown): string | undefined => {
      if (typeof v === "string") return v;
      if (typeof v === "object" && v !== null) {
        const c = v as Record<string, unknown>;
        for (const key of ["browser", "import", "module", "default"]) {
          const hit = pick(c[key]);
          if (hit) return hit;
        }
      }
      return undefined;
    };
    const hit = pick(dot ?? manifest.exports);
    if (hit) return hit.replace(/^\.\//, "");
  }
  return (manifest.module ?? manifest.main ?? "index.js").replace(/^\.\//, "");
}

/**
 * Executes a consented {@link DownloadPlan}: fetches the tarball, verifies
 * the registry integrity, and bundles the package into a single ESM file
 * destined for the project (`vendor/<pkg>.js`). The caller adds the file to
 * the project, the alias to the build, and the record to
 * `runtime.bundled_deps`.
 */
export async function downloadAndBundle(
  plan: DownloadPlan,
  engine: EsbuildApi,
  fetchImpl: FetchLike,
  aliases?: Record<string, string>,
  aliasFiles?: Map<string, Uint8Array>,
): Promise<ResolvedDependency> {
  const res = await fetchImpl(plan.tarball);
  if (!res.ok) throw new Error(`tarball download failed: HTTP ${res.status}`);
  const bytes = new Uint8Array(await res.arrayBuffer());
  if ((await sha512base64(bytes)) !== plan.integrity) throw new IntegrityError(plan.pkg);

  const entries = untar(await gunzip(bytes));
  const files = new Map<string, Uint8Array>();
  let manifest: { exports?: unknown; module?: string; main?: string } = {};
  for (const entry of entries) {
    // Tarballs root at `package/` (normalized name for the odd ones out).
    const rel = entry.path.split("/").slice(1).join("/");
    if (rel === "") continue;
    files.set(`vendor-src/${rel}`, entry.content);
    if (rel === "package.json") {
      manifest = JSON.parse(new TextDecoder().decode(entry.content)) as typeof manifest;
    }
  }

  const entry = `vendor-src/${entryOf(manifest)}`;
  if (!files.has(entry)) {
    throw new Error(`\`${plan.pkg}\` has no resolvable entry (looked for ${entry})`);
  }
  for (const [path, content] of aliasFiles ?? []) files.set(path, content);
  const input: BundleInput = { files, entry };
  if (aliases) input.aliases = aliases;
  const out = await bundle(engine, input);

  const safe = plan.pkg.replace("/", "__");
  return {
    file: `vendor/${safe}.js`,
    content: out.js,
    record: `${plan.pkg}@${plan.version}`,
    specifier: plan.specifier,
  };
}
