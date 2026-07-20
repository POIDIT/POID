/**
 * Serves container contents to the sandboxed iframe from an opaque origin
 * (SPEC §5.2). The resolver logic — path → response, MIME type, refusing
 * anything outside the container, injecting the SDK bootstrap and CSP into the
 * entry HTML — is here and unit-tested. The actual synthetic origin is a
 * service worker (browser-only), wired in the reader window and exercised by
 * the browser test tier.
 *
 * The application is served relative URLs that resolve within this synthetic
 * origin; it can never reach the real container file, the vault, the host
 * page, or the filesystem.
 */

import type { BootstrapConfig } from "@poid/sdk";
import { buildCsp, type CspOptions } from "./csp.js";

/** A resolved response for one container path. */
export interface ServedResponse {
  status: number;
  contentType: string;
  body: Uint8Array;
}

const MIME: Record<string, string> = {
  html: "text/html; charset=utf-8",
  js: "text/javascript; charset=utf-8",
  mjs: "text/javascript; charset=utf-8",
  css: "text/css; charset=utf-8",
  json: "application/json; charset=utf-8",
  svg: "image/svg+xml",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  woff2: "font/woff2",
  wasm: "application/wasm",
  txt: "text/plain; charset=utf-8",
};

function mimeFor(path: string): string {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  return MIME[ext] ?? "application/octet-stream";
}

const HTML_MIME = MIME.html ?? "text/html; charset=utf-8";
const TXT_MIME = MIME.txt ?? "text/plain; charset=utf-8";

/** How a container is presented to the resolver: its files, its entry path,
 * the SDK source to inject, and the bootstrap + CSP to embed. */
export interface ContainerServerInput {
  /** Container path → bytes (as produced by `poid-core`, `app/…`). */
  files: Map<string, Uint8Array>;
  /** Manifest `entry`, e.g. `app/index.html`. */
  entry: string;
  /** The `@poid/sdk` sandbox bootstrap, bundled to a self-executing IIFE. It
   * installs `window.poid` when the injected script runs. */
  sdkSource: string;
  /** Capabilities + app info the SDK reads synchronously at bootstrap. */
  bootstrap: BootstrapConfig;
  /** Approved network origins for the CSP `connect-src`. */
  csp?: CspOptions;
}

const encoder = new TextEncoder();

/** Maps container paths to responses, injecting the runtime into the entry. */
export class ContainerServer {
  private readonly files: Map<string, Uint8Array>;
  private readonly entry: string;
  private readonly entryHtml: Uint8Array;

  constructor(input: ContainerServerInput) {
    this.files = input.files;
    this.entry = normalize(input.entry);
    const raw = this.files.get(this.entry);
    if (!raw) throw new Error(`entry \`${input.entry}\` not found in container`);
    this.entryHtml = encoder.encode(
      injectRuntime(new TextDecoder().decode(raw), input.sdkSource, input.bootstrap, input.csp),
    );
  }

  /** The entry path, served at the synthetic origin root. */
  get entryPath(): string {
    return this.entry;
  }

  /**
   * Every asset the synthetic origin serves: the injected entry HTML plus each
   * container file at its own path. This is what a reader hands to its origin
   * mechanism (the desktop `poid://` protocol, the web service worker) so a
   * relative `<script src="app/main.js">` resolves to real bytes. The map is
   * the single source of truth — both readers serve exactly this.
   */
  assets(): Map<string, ServedResponse> {
    const out = new Map<string, ServedResponse>();
    out.set(this.entry, {
      status: 200,
      contentType: HTML_MIME,
      body: this.entryHtml,
    });
    for (const [path, body] of this.files) {
      const key = normalize(path);
      if (key === this.entry) continue;
      out.set(key, { status: 200, contentType: mimeFor(key), body });
    }
    return out;
  }

  /**
   * Resolves a request path within the container. Returns 404 for anything not
   * present; the synthetic origin never exposes the vault, the raw container,
   * or the host — only these bytes.
   */
  resolve(rawPath: string): ServedResponse {
    const path = normalize(rawPath);
    if (path === this.entry || path === "" || path === "index.html") {
      return { status: 200, contentType: HTML_MIME, body: this.entryHtml };
    }
    const body = this.files.get(path);
    if (!body) return { status: 404, contentType: TXT_MIME, body: encoder.encode("not found") };
    return { status: 200, contentType: mimeFor(path), body };
  }
}

/** Normalises a request path to a container key: strips a leading slash and a
 * query/fragment, and refuses traversal. `poid-core` already rejected traversal
 * at open time, so this is defense in depth. */
function normalize(path: string): string {
  let p = path.split("?")[0]?.split("#")[0] ?? "";
  if (p.startsWith("/")) p = p.slice(1);
  if (p.split("/").some((seg) => seg === ".." || seg === ".")) return "\0invalid";
  return p;
}

/**
 * Injects the CSP meta and the SDK bootstrap as the first things in the
 * document `<head>`, so `window.poid` exists before any application script
 * runs. The bootstrap global is embedded as JSON and consumed (then deleted)
 * by `installPoid`.
 */
export function injectRuntime(
  html: string,
  sdkSource: string,
  bootstrap: BootstrapConfig,
  csp: CspOptions = {},
): string {
  const cspMeta = `<meta http-equiv="Content-Security-Policy" content="${buildCsp(csp)}">`;
  const bootstrapJson = JSON.stringify(bootstrap).replace(/</g, "\\u003c");
  // The SDK source is a self-executing IIFE that installs `window.poid`; the
  // bootstrap global must be set before it runs.
  const injected =
    `${cspMeta}\n` +
    `<script>window.__POID_BOOTSTRAP__=${bootstrapJson};</script>\n` +
    `<script>\n${sdkSource}\n</script>\n`;

  const headIdx = html.search(/<head[^>]*>/i);
  if (headIdx !== -1) {
    const insertAt = html.indexOf(">", headIdx) + 1;
    return `${html.slice(0, insertAt)}\n${injected}${html.slice(insertAt)}`;
  }
  // No <head>: prepend a minimal one, moving any leading doctype ahead of it
  // so the document has exactly one, at the top.
  const doctype = html.match(/^\s*<!doctype[^>]*>/i);
  const rest = doctype ? html.slice(doctype[0].length) : html;
  return `${doctype ? doctype[0] : "<!doctype html>"}<head>\n${injected}</head>${rest}`;
}
