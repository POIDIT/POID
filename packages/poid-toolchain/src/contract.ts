/**
 * The build contract: the single pinned esbuild version and option set shared
 * by every build path (ARCHITECTURE §5.1). The JSON file is the source of
 * truth — this module types it and shapes it for the esbuild JS API; the CLI
 * (`crates/poid-cli`) embeds the same file and shapes it into flags. A parity
 * test builds the same input through the native binary and the WASM engine
 * and asserts byte-identical output.
 */

import contractJson from "./build-contract.json" with { type: "json" };

/** Asset/text loader assignments by file extension. */
export type LoaderMap = Record<string, "dataurl" | "text">;

/** The typed build contract. */
export interface BuildContract {
  /** Exact esbuild version; both engines MUST match it (reproducibility). */
  esbuild: string;
  format: "esm";
  platform: "browser";
  target: string;
  charset: "utf8";
  legalComments: "none";
  jsx: "automatic";
  minify: boolean;
  /** Fixed entry basename for outputs (`main.js` / `main.css`). */
  entryName: string;
  define: Record<string, string>;
  loader: LoaderMap;
}

/** The contract, as committed in `build-contract.json`. */
export const CONTRACT: BuildContract = contractJson as BuildContract;

/** The exact esbuild version every engine must run. */
export const PINNED_ESBUILD = CONTRACT.esbuild;

/**
 * The contract as esbuild JS-API options (native `esbuild` and `esbuild-wasm`
 * accept the same shape). `entryPoints`, `outdir`, `absWorkingDir`, plugins
 * and aliases are call-site concerns and deliberately not part of this
 * object.
 */
export function contractBuildOptions(): {
  bundle: true;
  format: "esm";
  platform: "browser";
  target: string;
  charset: "utf8";
  legalComments: "none";
  jsx: "automatic";
  minify: boolean;
  entryNames: string;
  define: Record<string, string>;
  loader: LoaderMap;
  write: false;
} {
  return {
    bundle: true,
    format: CONTRACT.format,
    platform: CONTRACT.platform,
    target: CONTRACT.target,
    charset: CONTRACT.charset,
    legalComments: CONTRACT.legalComments,
    jsx: CONTRACT.jsx,
    minify: CONTRACT.minify,
    entryNames: CONTRACT.entryName,
    define: { ...CONTRACT.define },
    loader: { ...CONTRACT.loader },
    write: false,
  };
}

/**
 * The contract as native-CLI flags, in a fixed order. `crates/poid-cli`
 * generates the same list from the same JSON; a unit test on each side pins
 * the expected sequence so the two cannot drift silently.
 */
export function contractCliFlags(): string[] {
  const flags = [
    "--bundle",
    `--format=${CONTRACT.format}`,
    `--platform=${CONTRACT.platform}`,
    `--target=${CONTRACT.target}`,
    `--charset=${CONTRACT.charset}`,
    `--legal-comments=${CONTRACT.legalComments}`,
    `--jsx=${CONTRACT.jsx}`,
  ];
  if (CONTRACT.minify) flags.push("--minify");
  flags.push(`--entry-names=${CONTRACT.entryName}`);
  // Sorted, so the sequence matches the Rust side's BTreeMap order exactly.
  for (const [key, value] of Object.entries(CONTRACT.define).sort()) {
    flags.push(`--define:${key}=${value}`);
  }
  for (const [ext, loader] of Object.entries(CONTRACT.loader).sort()) {
    flags.push(`--loader:${ext}=${loader}`);
  }
  return flags;
}
