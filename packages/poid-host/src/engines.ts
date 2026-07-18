/**
 * Reader-provided language engines (SPEC §5.4): the reader carries the
 * engine, the file carries the application and its wheels. Readers MUST
 * verify engine integrity before use, and applications can never trigger an
 * engine download — the reader either has a verified engine or it honestly
 * says it does not.
 *
 * The engine manifest shape (name / version / compatible / per-file sha256)
 * is the working draft for the SPEC engines chapter; `engines/pyodide.json`
 * in this repository is the pinned instance the tests run against.
 */

/** A pinned engine identity, as committed in `engines/<name>.json`. */
export interface EngineManifest {
  name: string;
  /** Exact engine version the reader ships. */
  version: string;
  /** The manifest `runtime.engines` range this build satisfies. */
  compatible: string;
  /** Relative file → sha256 (lowercase hex) of every file the engine runs. */
  files: Record<string, string>;
}

/** Raised when an engine file does not match its pinned checksum. */
export class EngineIntegrityError extends Error {
  constructor(file: string) {
    super(
      `engine file \`${file}\` does not match its pinned checksum; ` +
        "refusing to run an unverified engine (SPEC 5.4)",
    );
    this.name = "EngineIntegrityError";
  }
}

async function sha256hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes.slice().buffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Fetches and verifies **every** file of an engine against its manifest.
 * Only after this resolves may the engine be executed from `baseUrl` —
 * a reader that loads first and checks later is not verifying anything.
 */
export async function verifyEngine(
  baseUrl: string,
  manifest: EngineManifest,
  fetchImpl: (url: string) => Promise<{ ok: boolean; arrayBuffer(): Promise<ArrayBuffer> }>,
): Promise<void> {
  for (const [file, expected] of Object.entries(manifest.files)) {
    const res = await fetchImpl(`${baseUrl}${file}`);
    if (!res.ok) throw new EngineIntegrityError(file);
    const digest = await sha256hex(new Uint8Array(await res.arrayBuffer()));
    if (digest !== expected) throw new EngineIntegrityError(file);
  }
}

/**
 * Does the reader's engine satisfy the manifest's requested range
 * (SPEC §3.1 `runtime.engines`, semver ranges like `>=0.26 <0.28`)?
 * Comparators and `^`/`~` cover what manifests actually write; anything
 * unparseable is a mismatch, never a silent pass.
 */
export function engineSatisfies(version: string, range: string): boolean {
  const v = parseVersion(version);
  if (!v) return false;
  return range.split("||").some((alternative) => {
    const tokens = alternative
      .trim()
      .replace(/(>=|<=|>|<|=|\^|~)\s+/g, "$1")
      .split(/\s+/)
      .filter(Boolean);
    if (tokens.length === 0) return false;
    return tokens.every((token) => {
      const m = /^(>=|<=|>|<|=|\^|~)?(\d+)(?:\.(\d+))?(?:\.(\d+))?$/.exec(token);
      if (!m) return false;
      const base: [number, number, number] = [Number(m[2]), Number(m[3] ?? 0), Number(m[4] ?? 0)];
      const c = compare(v, base);
      switch (m[1]) {
        case ">=":
          return c >= 0;
        case "<=":
          return c <= 0;
        case ">":
          return c > 0;
        case "<":
          return c < 0;
        case "~":
          return c >= 0 && v[0] === base[0] && v[1] === base[1];
        case "^":
          if (c < 0) return false;
          if (base[0] > 0) return v[0] === base[0];
          if (base[1] > 0) return v[0] === 0 && v[1] === base[1];
          return c === 0;
        default:
          if (m[4] !== undefined) return c === 0;
          if (m[3] !== undefined) return v[0] === base[0] && v[1] === base[1];
          return v[0] === base[0];
      }
    });
  });
}

function parseVersion(version: string): [number, number, number] | undefined {
  const m = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined;
}

function compare(a: [number, number, number], b: [number, number, number]): number {
  for (let i = 0; i < 3; i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}
