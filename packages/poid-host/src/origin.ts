/**
 * The synthetic origin (SPEC §5.2.1): where a reader serves the application
 * and its relative subresources so a multi-file app's `<script src>` executes,
 * without ever exposing the container file, the vault, or the host.
 *
 * The reader builds the served assets once (from [`ContainerServer.assets`])
 * and hands them to a `SyntheticOrigin`. Each platform implements the origin
 * with the primitive that gives it a real, isolated origin:
 *   - desktop: a Tauri custom URI-scheme protocol (`poid://<session>/…`),
 *   - web: a service worker serving a per-session scope.
 *
 * A reader with no synthetic origin (fully offline `file://`, or a test)
 * falls back to [`BlobOrigin`]: a single blob document. Blob documents have
 * an opaque origin with no name a CSP `script-src` can reference, so only the
 * entry's inline scripts run — M04/M06 single-file parity. That is a genuine
 * limitation, surfaced in the UI, not a silent degradation.
 */

import type { ServedResponse } from "./container-server.js";

/** Serves one session's assets from an isolated, reader-controlled origin. */
export interface SyntheticOrigin {
  /**
   * The CSP source the entry document must name so its own subresources load
   * (SPEC §5.2.1): a bare `scheme:` or `scheme://host`. Empty for the blob
   * fallback (inline-only). The reader bakes this into the CSP before serving.
   */
  cspAssetSource(): string;

  /**
   * Registers a session's assets and returns the URL the iframe should load
   * (the injected entry document at the session's origin root).
   */
  serve(sessionId: string, assets: Map<string, ServedResponse>, entryPath: string): Promise<string>;

  /** Releases a session's assets (called on reader teardown). */
  revoke(sessionId: string): void;
}

/**
 * The blob fallback: one document, opaque origin, subresources do not resolve.
 * Used when no real synthetic origin is available. `revoke` frees the object
 * URL it minted.
 */
export class BlobOrigin implements SyntheticOrigin {
  private readonly urls = new Map<string, string>();

  constructor(private readonly win: { URL: typeof URL } = globalThis) {}

  cspAssetSource(): string {
    // A blob document is opaque; there is no origin subresources could name.
    return "";
  }

  serve(
    sessionId: string,
    assets: Map<string, ServedResponse>,
    entryPath: string,
  ): Promise<string> {
    const entry = assets.get(entryPath);
    if (!entry) throw new Error(`entry \`${entryPath}\` missing from served assets`);
    // Decode to text: the entry is always HTML, and a string BlobPart sidesteps
    // the Uint8Array/ArrayBufferLike typing friction.
    const html = new TextDecoder().decode(entry.body);
    const blob = new Blob([html], { type: "text/html" });
    const url = this.win.URL.createObjectURL(blob);
    this.urls.set(sessionId, url);
    return Promise.resolve(url);
  }

  revoke(sessionId: string): void {
    const url = this.urls.get(sessionId);
    if (url) {
      this.win.URL.revokeObjectURL(url);
      this.urls.delete(sessionId);
    }
  }
}
