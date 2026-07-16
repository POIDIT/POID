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
  return [
    "default-src 'self'",
    connect,
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
    // Scripts and styles come from the container's opaque origin; inline is
    // needed for the injected bootstrap and app markup.
    "script-src 'self' 'unsafe-inline'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
  ].join("; ");
}

/** The sandbox token set for the application iframe. Deliberately **without**
 * `allow-same-origin`: the app runs in an opaque origin and cannot reach the
 * container file, the vault, or the host (SPEC §5.2, SECURITY §3). */
export const SANDBOX_TOKENS = "allow-scripts" as const;
