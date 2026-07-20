/**
 * The Web Reader's synthetic origin (SPEC §5.2.1): serves the app's
 * subresources as `data:` URLs so a multi-file app's `<script src>` executes.
 *
 * Why not a service worker? It does **not** control sandboxed, opaque-origin
 * clients — the very isolation the application iframe requires
 * (`sandbox="allow-scripts"`, no `allow-same-origin`). Verified: the page's
 * own fetch of a worker-served URL succeeds, but the sandboxed iframe's
 * navigation to the same URL bypasses the worker and 404s.
 *
 * Why `data:` and not `blob:`? A `blob:` URL is scoped to the origin that
 * minted it (the reader page); the sandbox's opaque origin cannot load it
 * ("Not allowed to load local resource"). `data:` URLs are not origin-scoped,
 * so the opaque document loads them, and they carry no CORS requirement.
 *
 * The reader rewrites the entry document's relative `src`/`href` references to
 * `data:` URLs of the container files. This keeps the opaque origin, works
 * fully offline and from `file://`, and needs no second origin. It resolves
 * references in HTML attributes; every POID is bundled by the toolchain
 * (esbuild `--bundle`, one `main.js`, no inter-chunk imports), which is
 * exactly what this serves losslessly. References buried in CSS `url()` or
 * runtime dynamic imports are out of scope — narrower than the desktop
 * `poid://` origin, and noted in the Web Reader's honest-limitations.
 */

import type { ServedResponse, SyntheticOrigin } from "@poid/host";

/** Base64-encodes bytes in chunks (a whole-array spread overflows for large
 * assets). */
function base64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** A `data:` URL for a served response (base64 to survive binary bytes). */
function dataUrl(res: ServedResponse): string {
  const mime = res.contentType.split(";")[0]?.trim() || "application/octet-stream";
  return `data:${mime};base64,${base64(res.body)}`;
}

/** Resolves `ref` (a relative URL) against `dir` (a container directory like
 * `app/`), collapsing `.`/`..`, to a container path key. */
function resolveRef(dir: string, ref: string): string {
  const stack = dir.split("/").filter(Boolean);
  for (const seg of ref.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") stack.pop();
    else stack.push(seg);
  }
  return stack.join("/");
}

export class BlobRewriteOrigin implements SyntheticOrigin {
  private readonly entryUrls = new Map<string, string>();

  cspAssetSource(): string {
    // Subresources are data: URLs of the container's own bytes; nothing else.
    return "data:";
  }

  serve(
    sessionId: string,
    assets: Map<string, ServedResponse>,
    entryPath: string,
  ): Promise<string> {
    const dataFor = new Map<string, string>();
    for (const [path, res] of assets) {
      if (path === entryPath) continue;
      dataFor.set(path, dataUrl(res));
    }

    const entry = assets.get(entryPath);
    if (!entry) throw new Error(`entry \`${entryPath}\` missing from served assets`);
    const dir = entryPath.includes("/") ? `${entryPath.slice(0, entryPath.lastIndexOf("/"))}/` : "";
    const html = new TextDecoder().decode(entry.body);

    // Rewrite relative `src`/`href` element attributes to their data: URLs.
    // DOMParser (not a regex) so only real attributes are touched — the
    // injected inline bootstrap/SDK scripts, whose bodies contain `src`-like
    // substrings, are left byte-exact.
    const doc = new DOMParser().parseFromString(html, "text/html");
    for (const el of doc.querySelectorAll("[src],[href]")) {
      for (const attr of ["src", "href"]) {
        const ref = el.getAttribute(attr);
        if (
          !ref ||
          /^[a-z][a-z0-9+.-]*:/i.test(ref) ||
          ref.startsWith("//") ||
          ref.startsWith("#")
        ) {
          continue;
        }
        const url = dataFor.get(resolveRef(dir, ref));
        if (url) el.setAttribute(attr, url);
      }
    }
    const rewritten = `<!doctype html>${doc.documentElement.outerHTML}`;

    // The entry document itself is a blob: URL — the iframe navigates to it,
    // which is allowed (the parent sets iframe.src), and its opaque origin
    // then loads the origin-agnostic data: subresources.
    const entryUrl = URL.createObjectURL(new Blob([rewritten], { type: "text/html" }));
    this.entryUrls.set(sessionId, entryUrl);
    return Promise.resolve(entryUrl);
  }

  revoke(sessionId: string): void {
    const url = this.entryUrls.get(sessionId);
    if (url) {
      URL.revokeObjectURL(url);
      this.entryUrls.delete(sessionId);
    }
  }
}
