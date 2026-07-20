/**
 * `@poid/web` — The Web Reader: a static, 100% client-side SPA for
 * recipients who have not installed Studio. The entry point of the site is
 * `src/main.ts` (bundled by `build:site`); this module exports the testable
 * building blocks.
 */

// Reader logic shared with the desktop Reader lives in `@poid/host` since M07;
// re-exported here so the Web Reader's public surface is unchanged.
export {
  consentManifestFrom,
  explain,
  extractFacts,
  FALLBACK_EXPLANATION,
  hostFacts,
  IndexedDbEngine,
  REGISTRY_EXPLANATIONS,
  type ReaderManifestFacts as WebManifestFacts,
  runGrant,
} from "@poid/host";
export { mountBanner } from "./banner.js";
export { triggerDownload, updatedFileBytes } from "./download.js";
export {
  type NoticeOutcome,
  type OpenOutcome,
  openBytes,
  type RejectedOutcome,
  type RunnableOutcome,
  type RunSession,
  runContainer,
} from "./open-flow.js";
export { VaultWasmEngine } from "./vault-engine.js";
export {
  type ContainerErrorLike,
  isContainerError,
  loadPoidWasm,
  type PoidWasm,
  type WebPoidHandle,
} from "./wasm-api.js";
