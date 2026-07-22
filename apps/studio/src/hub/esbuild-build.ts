/**
 * Running esbuild-wasm inside the hub window (M12.2b).
 *
 * The build engine is a downloaded runtime: Rust fetches, verifies and caches
 * the 11.76 MiB wasm, and hands it here as bytes. We compile those bytes into
 * a `WebAssembly.Module` and initialise esbuild with it — `wasmModule` rather
 * than `wasmURL`, so nothing is fetched over a URL the window's CSP would have
 * to allow. `worker: false` keeps esbuild on the main thread, which a
 * document-application window has no worker-src grant for anyway.
 *
 * The bundle itself is `@poid/toolchain`'s: the same in-memory build the CLI's
 * sidecar performs, over the files and aliases the converter's Rust side
 * staged. This module only supplies the engine.
 */

import { type BundleOutput, bundle } from "@poid/toolchain";
import { invoke } from "@tauri-apps/api/core";

/** The esbuild-wasm API once initialised: version plus build. */
interface EsbuildApi {
  version: string;
  build(options: object): Promise<{
    outputFiles?: { path: string; contents: Uint8Array }[];
    errors: { text: string }[];
  }>;
  initialize(options: { wasmModule?: WebAssembly.Module; worker?: boolean }): Promise<void>;
}

/** Decodes base64 to bytes. */
function fromBase64(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** base64 of a byte array, chunked to spare the call stack. */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/**
 * Initialised once per window: esbuild-wasm's initialise() must not run twice,
 * and compiling 11.76 MiB is not worth repeating. Concurrent callers await the
 * same promise.
 */
let enginePromise: Promise<EsbuildApi> | undefined;

/** The engine, downloading and compiling it on first use. `onProgress` is told
 * when the (potentially slow, potentially networked) engine fetch is under way
 * so the UI can say so. */
async function engine(onProgress: (message: string) => void): Promise<EsbuildApi> {
  if (!enginePromise) {
    enginePromise = (async () => {
      onProgress("Getting the build engine ready…");
      const wasmBase64 = await invoke<string>("esbuild_engine_wasm");
      const wasmModule = await WebAssembly.compile(fromBase64(wasmBase64).buffer as ArrayBuffer);
      // esbuild-wasm's browser build is bundled into this window; import it
      // dynamically so the engine only loads when a build is actually needed.
      const esbuild = (await import("esbuild-wasm")) as unknown as EsbuildApi;
      await esbuild.initialize({ wasmModule, worker: false });
      return esbuild;
    })().catch((e) => {
      // A failed init must not poison every later attempt.
      enginePromise = undefined;
      throw e;
    });
  }
  return enginePromise;
}

/** The build inputs `convert_prepare` returns for a project that needs one. */
export interface BuildRequest {
  entry: string;
  buildFiles: { rel: string; bytesBase64: string }[];
  aliases: Record<string, string>;
}

/** The bundle, base64-encoded for the trip back to `convert_finish`. */
export interface BuiltBundle {
  jsBase64: string;
  cssBase64: string | null;
}

/** Builds a prepared project with esbuild-wasm and returns the bundle. */
export async function buildInApp(
  request: BuildRequest,
  onProgress: (message: string) => void,
): Promise<BuiltBundle> {
  const esbuild = await engine(onProgress);
  onProgress("Building…");

  const files = new Map<string, Uint8Array>();
  for (const f of request.buildFiles) files.set(f.rel, fromBase64(f.bytesBase64));

  const output: BundleOutput = await bundle(esbuild, {
    files,
    entry: request.entry,
    aliases: request.aliases,
  });

  return {
    jsBase64: toBase64(output.js),
    cssBase64: output.css ? toBase64(output.css) : null,
  };
}
