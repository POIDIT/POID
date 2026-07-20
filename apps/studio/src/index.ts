/**
 * `@poid/studio` — POID Studio's window frontends (Tauri v2).
 *
 * The entry points are `reader-main.ts` (a Reader window) and `hub-main.ts`
 * (the Studio hub), bundled by `scripts/build-ui.mjs`. This module exports
 * the testable building blocks.
 */

export {
  type DesktopOutcome,
  type NoticeOutcome,
  type RejectedOutcome,
  type RunnableOutcome,
  routeDocument,
} from "./desktop-flow.js";
export { type DocumentDto, decodeFiles, type FileEntryDto } from "./document-dto.js";
