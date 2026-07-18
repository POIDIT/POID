/**
 * The in-app build engine: esbuild running over an in-memory file map — the
 * shape Studio and the Web Reader work with, where there is no filesystem.
 * The engine implementation is injected (esbuild-wasm in the app, native
 * esbuild in the parity test), so this module never chooses an engine and
 * never touches the network.
 *
 * Everything semantic — options, loaders, the pinned version — comes from the
 * build contract; this module only adapts it to a virtual filesystem.
 */

import { CONTRACT, contractBuildOptions, PINNED_ESBUILD } from "./contract.js";

/** The slice of the esbuild JS API the engine needs (native and wasm agree). */
export interface EsbuildApi {
  version: string;
  build(options: object): Promise<{
    outputFiles?: { path: string; contents: Uint8Array }[];
    errors: { text: string }[];
  }>;
}

/** One build request over in-memory sources. */
export interface BundleInput {
  /** Project files: relative path (forward slashes) → content. */
  files: Map<string, Uint8Array>;
  /** Entry point, e.g. `src/main.tsx`. Must exist in `files`. */
  entry: string;
  /** Bare-specifier aliases (Standard Library / resolver), e.g.
   * `react` → `stdlib/react.js` (a path present in `files`). */
  aliases?: Record<string, string>;
}

/** The bundle produced by one build. */
export interface BundleOutput {
  /** The bundled, minified ESM. */
  js: Uint8Array;
  /** Bundled CSS, when any stylesheet was imported. */
  css?: Uint8Array;
  /** The engine version that produced this output (`runtime.toolchain`). */
  esbuildVersion: string;
}

/** Thrown when the injected engine does not match the pinned version —
 * reproducibility is a promise, not a preference. */
export class EngineVersionError extends Error {
  constructor(found: string) {
    super(
      `esbuild ${found} does not match the pinned ${PINNED_ESBUILD} from the build contract; ` +
        "builds would not be reproducible",
    );
    this.name = "EngineVersionError";
  }
}

/** Thrown when the bundle fails; `messages` are esbuild's own diagnostics. */
export class BundleError extends Error {
  readonly messages: string[];
  constructor(messages: string[]) {
    super(messages.join("\n"));
    this.name = "BundleError";
    this.messages = messages;
  }
}

const decoder = new TextDecoder();

function normalize(p: string): string {
  const out: string[] = [];
  for (const seg of p.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") out.pop();
    else out.push(seg);
  }
  return out.join("/");
}

function dirname(p: string): string {
  const i = p.lastIndexOf("/");
  return i === -1 ? "" : p.slice(0, i);
}

function extension(p: string): string {
  const i = p.lastIndexOf(".");
  return i === -1 ? "" : p.slice(i);
}

/**
 * Bundles `input.entry` from the in-memory file map with the contract
 * options. Bare specifiers resolve only through `input.aliases`; anything
 * else is a build error (the Standard Library and the Resolver decide what
 * goes into `aliases` — the engine never guesses and never fetches).
 */
export async function bundle(engine: EsbuildApi, input: BundleInput): Promise<BundleOutput> {
  if (engine.version !== PINNED_ESBUILD) throw new EngineVersionError(engine.version);
  if (!input.files.has(input.entry)) {
    throw new BundleError([`entry \`${input.entry}\` is not in the project file map`]);
  }

  const virtualFs = {
    name: "poid-virtual-fs",
    setup: (build: {
      onResolve(
        filter: { filter: RegExp },
        cb: (args: {
          path: string;
          importer: string;
          kind: string;
        }) => { path: string; namespace: string } | { errors: { text: string }[] } | undefined,
      ): void;
      onLoad(
        filter: { filter: RegExp; namespace: string },
        cb: (args: { path: string }) => { contents: Uint8Array; loader: string } | undefined,
      ): void;
    }) => {
      build.onResolve({ filter: /.*/ }, (args) => {
        // Entry point: resolved by us, loaded from the map.
        if (args.kind === "entry-point") {
          return { path: normalize(args.path), namespace: "poid" };
        }
        // Relative imports resolve against the importer's directory.
        if (args.path.startsWith("./") || args.path.startsWith("../")) {
          const base = args.importer ? dirname(args.importer) : "";
          const resolved = normalize(`${base}/${args.path}`);
          const candidates = [
            resolved,
            `${resolved}.ts`,
            `${resolved}.tsx`,
            `${resolved}.js`,
            `${resolved}.jsx`,
            `${resolved}/index.ts`,
            `${resolved}/index.tsx`,
            `${resolved}/index.js`,
          ];
          const hit = candidates.find((c) => input.files.has(c));
          if (hit) return { path: hit, namespace: "poid" };
          return {
            errors: [{ text: `cannot resolve \`${args.path}\` from \`${args.importer}\`` }],
          };
        }
        // Bare specifiers: aliases only. The message mirrors the CLI's.
        const alias = input.aliases?.[args.path];
        if (alias && input.files.has(alias)) return { path: alias, namespace: "poid" };
        return {
          errors: [
            {
              text:
                `cannot resolve bare import \`${args.path}\`. It is not in the Standard ` +
                "Library selection for this build and was not bundled by the Resolver.",
            },
          ],
        };
      });

      build.onLoad({ filter: /.*/, namespace: "poid" }, (args) => {
        const contents = input.files.get(args.path);
        if (!contents) return undefined;
        const ext = extension(args.path);
        const custom = CONTRACT.loader[ext];
        const loader =
          custom ??
          (ext === ".ts"
            ? "ts"
            : ext === ".tsx"
              ? "tsx"
              : ext === ".jsx"
                ? "jsx"
                : ext === ".css"
                  ? args.path.endsWith(".module.css")
                    ? "local-css"
                    : "css"
                  : ext === ".json"
                    ? "json"
                    : "js");
        return { contents, loader };
      });
    },
  };

  const result = await engine.build({
    ...contractBuildOptions(),
    entryPoints: [input.entry],
    outdir: "out",
    plugins: [virtualFs],
  });

  if (result.errors.length > 0) {
    throw new BundleError(result.errors.map((e) => e.text));
  }

  let js: Uint8Array | undefined;
  let css: Uint8Array | undefined;
  for (const file of result.outputFiles ?? []) {
    if (file.path.endsWith(`${CONTRACT.entryName}.js`)) js = file.contents;
    if (file.path.endsWith(`${CONTRACT.entryName}.css`)) css = file.contents;
  }
  if (!js) throw new BundleError(["the build produced no JavaScript output"]);

  const output: BundleOutput = { js, esbuildVersion: engine.version };
  if (css) output.css = css;
  return output;
}

/** Decodes a bundle for callers that want strings (HTML inlining). */
export function bundleText(output: BundleOutput): { js: string; css?: string } {
  const out: { js: string; css?: string } = { js: decoder.decode(output.js) };
  if (output.css) out.css = decoder.decode(output.css);
  return out;
}
