/**
 * `@poid/web` — The Web Reader: a static, 100% client-side SPA for
 * recipients who have not installed Studio. The entry point of the site is
 * `src/main.ts` (bundled by `build:site`); this module exports the testable
 * building blocks.
 */

export { mountBanner } from "./banner.js";
export { triggerDownload, updatedFileBytes } from "./download.js";
export { explain, FALLBACK_EXPLANATION, REGISTRY_EXPLANATIONS } from "./errors.js";
export { IndexedDbEngine } from "./idb-engine.js";
export {
  consentManifestFrom,
  extractFacts,
  hostFacts,
  runGrant,
  type WebManifestFacts,
} from "./manifest-facts.js";
export {
  type NoticeOutcome,
  type OpenOutcome,
  openBytes,
  type RejectedOutcome,
  type RunnableOutcome,
  type RunSession,
  runContainer,
} from "./open-flow.js";
export {
  type ContainerErrorLike,
  isContainerError,
  loadPoidWasm,
  type PoidWasm,
  type WebPoidHandle,
} from "./wasm-api.js";
