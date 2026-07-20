/**
 * From an IPC document payload to a routing decision — the desktop twin of
 * the Web Reader's `openBytes` (`@poid/web`). Validation already happened in
 * `poid-core` on the Rust side; both readers then interpret the manifest with
 * the same `extractFacts` and apply the same routing rules, so a container
 * behaves identically on the desktop and on the web (M05: parity, honest
 * limitations).
 */

import { explain, extractFacts, type ReaderManifestFacts } from "@poid/host";
import { type DocumentDto, decodeFiles } from "./document-dto.js";

/** The container was refused by the validation core. */
export interface RejectedOutcome {
  kind: "rejected";
  registry: string | undefined;
  code: string;
  message: string;
  explanation: string;
  fileName: string;
}

/** The container is valid but is not a runnable app for this reader. */
export interface NoticeOutcome {
  kind: "data-container" | "workspace" | "engine-missing";
  facts: ReaderManifestFacts;
  missingEngines: string[];
  fileName: string;
}

/** A runnable `type: app`, `web` profile container. */
export interface RunnableOutcome {
  kind: "runnable";
  facts: ReaderManifestFacts;
  signature: "none" | "valid";
  files: Map<string, Uint8Array>;
  fileName: string;
}

/** The routing decision for one opened document. */
export type DesktopOutcome = RejectedOutcome | NoticeOutcome | RunnableOutcome;

/**
 * Routes a document payload. On the runnable path a missing `instance.id`
 * is assigned here (SPEC §6.3); writing it back into the file on disk is a
 * vault-milestone concern — for now, like the Web Reader, the id lives for
 * this instance's storage scope.
 */
export function routeDocument(dto: DocumentDto): DesktopOutcome {
  if (dto.kind === "rejected") {
    return {
      kind: "rejected",
      registry: dto.registry ?? undefined,
      code: dto.code,
      message: dto.message,
      explanation: explain(dto.registry ?? undefined),
      fileName: dto.fileName,
    };
  }

  const facts = extractFacts(dto.manifestJson);

  if (facts.type === "data" || facts.type === "workspace") {
    return {
      kind: facts.type === "data" ? "data-container" : "workspace",
      facts,
      missingEngines: [],
      fileName: dto.fileName,
    };
  }

  // Engines are provided by the reader, never the file (SPEC §5.4). The
  // desktop reader does not ship engine loading yet (the Pyodide wiring is a
  // follow-up), so any `web+<engine>` profile gets an honest notice instead
  // of a broken run — exactly like the Web Reader.
  if (facts.profile !== "web") {
    const missing = Object.keys(facts.engines);
    return {
      kind: "engine-missing",
      facts,
      missingEngines: missing.length > 0 ? missing : facts.profile.split("+").slice(1),
      fileName: dto.fileName,
    };
  }

  if (!facts.instanceId) {
    facts.instanceId = crypto.randomUUID();
  }

  return {
    kind: "runnable",
    facts,
    signature: dto.signature,
    files: decodeFiles(dto.files),
    fileName: dto.fileName,
  };
}
