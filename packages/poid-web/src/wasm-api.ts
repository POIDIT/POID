/**
 * Loader and types for the validation core: `poid-core` compiled to WASM via
 * `crates/poid-wasm`. The Web Reader runs **the same validation code as the
 * desktop reader** — nothing here re-implements a container check.
 *
 * The generated bindings (`src/wasm/`, produced by `build:wasm`) are not
 * committed and are imported with a computed specifier, so `tsc` and esbuild
 * never resolve them: typechecking works before the WASM exists, the browser
 * imports the module natively at runtime, and the Playwright tier exercises
 * the real bindings.
 */

/** A container rejection thrown by the core (mirrors `ContainerError`). */
export interface ContainerErrorLike {
  /** Stable diagnostic code, e.g. `native-code`. */
  readonly code: string;
  /** Normative registry code, e.g. `POID-020`; absent for environmental errors. */
  readonly registry: string | undefined;
  /** Human-readable explanation from the core. */
  readonly message: string;
}

/** A validated, opened container (mirrors `WebPoid`). */
export interface WebPoidHandle {
  manifestJson(): string;
  signatureStatus(): string;
  filePaths(): string[];
  file(path: string): Uint8Array | undefined;
  data(): Uint8Array | undefined;
  setInstanceId(id: string): void;
  setData(data: Uint8Array): void;
  toBytes(): Uint8Array;
  /** Releases the WASM-side memory; the handle is unusable afterwards. */
  free(): void;
}

/** One instance's CRDT memory (mirrors `VaultDoc` — the same Automerge
 * engine as the desktop vault, SPEC §6.5). Values cross as JSON strings. */
export interface VaultDocHandle {
  save(): Uint8Array;
  merge(other: Uint8Array): void;
  kvGet(slot: string, key: string): string | undefined;
  kvSet(slot: string, key: string, json: string): void;
  kvDelete(slot: string, key: string): void;
  kvList(slot: string, prefix?: string): string[];
  kvClear(slot: string): void;
  usage(slot: string): number;
  totalUsage(): number;
  slots(): string[];
  currentSlot(): string;
  setCurrentSlot(slot: string): void;
  /** Releases the WASM-side memory; the handle is unusable afterwards. */
  free(): void;
}

/** The constructor side of `VaultDoc`. */
export interface VaultDocClass {
  new (): VaultDocHandle;
  load(bytes: Uint8Array): VaultDocHandle;
}

/** The surface of the WASM module the Web Reader uses. */
export interface PoidWasm {
  openContainer(bytes: Uint8Array): WebPoidHandle;
  VaultDoc: VaultDocClass;
}

interface WasmModule extends PoidWasm {
  /** wasm-bindgen's async init; with no argument it fetches the `.wasm`
   * relative to the module's own URL. */
  default(): Promise<unknown>;
}

let instance: Promise<PoidWasm> | undefined;

/** Loads and instantiates the validation core. Idempotent — the module is
 * fetched and compiled once and cached for the life of the page. */
export function loadPoidWasm(): Promise<PoidWasm> {
  if (!instance) {
    const specifier = "./wasm/poid_wasm.js";
    instance = (import(specifier) as Promise<WasmModule>).then(async (mod) => {
      await mod.default();
      return mod;
    });
  }
  return instance;
}

/** Narrows an unknown thrown value to a container rejection. */
export function isContainerError(e: unknown): e is ContainerErrorLike {
  if (typeof e !== "object" || e === null) return false;
  const c = e as { code?: unknown; message?: unknown };
  return typeof c.code === "string" && typeof c.message === "string";
}
