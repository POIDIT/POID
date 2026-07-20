/**
 * The Content-Security-Policy applied to the sandboxed application (SPEC §5.2).
 *
 * The default posture forbids all network egress (`connect-src 'none'`). It is
 * widened only to origins the manifest declared **and** the user approved —
 * enforced by the browser, not by convention (SECURITY §3).
 */

/** Origins granted network access after user consent. */
export interface CspOptions {
  /** Approved origins from `permissions.network`; empty = no network at all. */
  connectSrc?: string[];
  /**
   * The synthetic origin's CSP source for the app's own subresources
   * (SPEC §5.2.1): a bare scheme-source (`poid:`) or origin
   * (`http://poid.localhost`). The application document has an **opaque**
   * origin (sandbox `allow-scripts`, no `allow-same-origin`), so `'self'`
   * matches nothing — `<script src="app/main.js">` only runs if `script-src`
   * names the origin that serves it. Empty (the blob fallback) means
   * inline-only: subresources do not load, single-file parity.
   */
  assetSource?: string;
}

/** Validates the synthetic-origin asset source: a bare `scheme:` or a bare
 * `scheme://host(:port)`, with nothing a CSP injection could smuggle. */
function isSafeAssetSource(source: string): boolean {
  if (/[;,'"\s]/.test(source)) return false;
  // A bare scheme-source, e.g. `poid:`.
  if (/^[a-z][a-z0-9+.-]*:$/i.test(source)) return true;
  try {
    const u = new URL(source);
    if (u.username || u.password || u.search || u.hash) return false;
    if (u.pathname !== "/" && u.pathname !== "") return false;
    return source === u.origin;
  } catch {
    return false;
  }
}

/** Validates that a granted network origin is a bare scheme+host(+port), so an
 * attacker cannot smuggle extra CSP directives through the allowlist. */
function isSafeOrigin(origin: string): boolean {
  try {
    const u = new URL(origin);
    if (u.protocol !== "https:" && u.protocol !== "http:") return false;
    // A bare origin has no path, query, fragment, credentials, or CSP-breaking
    // characters.
    if (u.username || u.password || u.search || u.hash) return false;
    if (u.pathname !== "/" && u.pathname !== "") return false;
    if (/[;,'"\s]/.test(origin)) return false;
    return origin === u.origin;
  } catch {
    return false;
  }
}

/**
 * Builds the CSP header/meta value for the application document. The five
 * baseline directives are always present and never relaxed; `connect-src` is
 * `'none'` unless approved origins are supplied.
 */
export function buildCsp(options: CspOptions = {}): string {
  const origins = (options.connectSrc ?? []).filter(isSafeOrigin);
  const connect = origins.length > 0 ? `connect-src ${origins.join(" ")}` : "connect-src 'none'";
  // The synthetic origin serving the app's subresources (SPEC §5.2.1). `'self'`
  // is kept for the blob fallback's data:/blob: cases and is harmless (it
  // matches nothing under an opaque origin); `'unsafe-inline'` keeps
  // M06-inlined single-file POIDs working.
  const asset =
    options.assetSource && isSafeAssetSource(options.assetSource) ? options.assetSource : "";
  const src = (extra: string) =>
    [`'self'`, "'unsafe-inline'", asset, extra].filter(Boolean).join(" ");
  return [
    "default-src 'self'",
    connect,
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
    `script-src ${src("")}`,
    `style-src ${src("")}`,
    `img-src ${src("data: blob:")}`,
  ].join("; ");
}

/** The sandbox token set for the application iframe. Deliberately **without**
 * `allow-same-origin`: the app runs in an opaque origin and cannot reach the
 * container file, the vault, or the host (SPEC §5.2, SECURITY §3). */
export const SANDBOX_TOKENS = "allow-scripts" as const;
