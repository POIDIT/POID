/**
 * From dropped bytes to a decision: validated by the WASM core, then routed
 * to the screen the container calls for. Only `type: app` with the plain
 * `web` profile executes; everything else gets an honest explanation —
 * never a silent failure, never a degraded run (M05: display limitations,
 * don't hide them).
 */

import { capabilitiesFromGrant, mountReader, type ReaderHandle, type Scope } from "@poid/host";
import { explain } from "./errors.js";
import { IndexedDbEngine } from "./idb-engine.js";
import {
  consentManifestFrom,
  extractFacts,
  hostFacts,
  runGrant,
  type WebManifestFacts,
} from "./manifest-facts.js";
import { isContainerError, loadPoidWasm, type WebPoidHandle } from "./wasm-api.js";

/** The container was rejected by the validation core. */
export interface RejectedOutcome {
  kind: "rejected";
  /** Normative registry code (`POID-xxx`), when the rejection has one. */
  registry: string | undefined;
  /** Stable diagnostic code, e.g. `native-code`. */
  code: string;
  /** Technical detail from the core. */
  message: string;
  /** Human-readable explanation for the error panel. */
  explanation: string;
}

/** The container is valid but is not a runnable app for this reader. */
export interface NoticeOutcome {
  kind: "data-container" | "workspace" | "engine-missing";
  facts: WebManifestFacts;
  /** For `engine-missing`: the engines the manifest calls for. */
  missingEngines: string[];
}

/** A runnable `type: app`, `web` profile container. */
export interface RunnableOutcome {
  kind: "runnable";
  facts: WebManifestFacts;
  signature: "none" | "valid";
  poid: WebPoidHandle;
  files: Map<string, Uint8Array>;
}

/** The routing decision for one opened file. */
export type OpenOutcome = RejectedOutcome | NoticeOutcome | RunnableOutcome;

/**
 * Validates container bytes and routes them. On the runnable path a missing
 * `instance.id` is assigned here (SPEC §6.3) — in the Web Reader it cannot be
 * written back to the original file; it travels in the downloaded copy.
 */
export async function openBytes(bytes: Uint8Array): Promise<OpenOutcome> {
  const wasm = await loadPoidWasm();

  let poid: WebPoidHandle;
  try {
    poid = wasm.openContainer(bytes);
  } catch (e) {
    if (isContainerError(e)) {
      return {
        kind: "rejected",
        registry: e.registry,
        code: e.code,
        message: e.message,
        explanation: explain(e.registry),
      };
    }
    throw e;
  }

  const facts = extractFacts(poid.manifestJson());
  const signature = poid.signatureStatus() === "valid" ? "valid" : "none";

  if (facts.type === "data" || facts.type === "workspace") {
    poid.free();
    return {
      kind: facts.type === "data" ? "data-container" : "workspace",
      facts,
      missingEngines: [],
    };
  }

  // Engines are provided by the reader, never the file (SPEC §5.4). This
  // reader ships none beyond the browser's own JS/WASM, so any `web+<engine>`
  // profile gets an honest notice instead of a broken run.
  if (facts.profile !== "web") {
    poid.free();
    const missing = Object.keys(facts.engines);
    return {
      kind: "engine-missing",
      facts,
      missingEngines: missing.length > 0 ? missing : facts.profile.split("+").slice(1),
    };
  }

  if (!facts.instanceId) {
    const id = crypto.randomUUID();
    poid.setInstanceId(id);
    facts.instanceId = id;
  }

  const files = new Map<string, Uint8Array>();
  for (const path of poid.filePaths()) {
    const content = poid.file(path);
    if (content) files.set(path, content);
  }

  return { kind: "runnable", facts, signature, poid, files };
}

/** A mounted (or consent-declined) reader session. */
export interface RunSession {
  handle: ReaderHandle;
  /** False when the user chose Cancel on the consent screen. */
  ran: boolean;
  engine: IndexedDbEngine;
  scope: Scope;
}

/**
 * Shows consent and, on Run, mounts the reader (`@poid/host` — the same
 * consent, sandbox, broker and watchdog as the desktop reader).
 *
 * The engine is hydrated (and, for `embedded` mode, seeded from
 * `data/store.json`) *before* consent, because the broker needs a live engine
 * at mount time; if the user cancels, a fresh seed is rolled back so a
 * declined app leaves no trace.
 */
export async function runContainer(
  outcome: RunnableOutcome,
  container: HTMLElement,
  sdkSource: string,
  engine?: IndexedDbEngine,
): Promise<RunSession> {
  const { facts, poid, files } = outcome;
  if (!facts.instanceId || !facts.entry) {
    // Unreachable: openBytes assigns the id and poid-core validated `entry`.
    throw new Error("runnable container without instance id or entry");
  }

  const db = engine ?? (await IndexedDbEngine.open());
  const scope: Scope = { instanceId: facts.instanceId, slot: "" };
  await db.hydrate(scope);

  let seeded = false;
  if (facts.storageMode === "embedded" && !facts.protectedData && db.isEmpty(scope)) {
    seeded = seedFromStore(db, scope, poid.data());
  }

  const handle = await mountReader({
    container,
    files,
    entry: facts.entry,
    manifest: {
      ...consentManifestFrom(facts, outcome.signature),
      instanceId: facts.instanceId,
      storageMode: facts.storageMode,
    },
    capabilities: capabilitiesFromGrant(hostFacts(facts), runGrant(facts)),
    connectSrc: facts.permissions.network,
    sdkSource,
    engine: db,
  });

  const ran = handle.chrome !== undefined;
  if (!ran && seeded) {
    db.kvClear(scope);
    await db.flush();
  }
  return { handle, ran, engine: db, scope };
}

/** Seeds the engine from an embedded `data/store.json`, if it parses to an
 * object (SPEC §6.2). Returns true when anything was seeded. */
function seedFromStore(db: IndexedDbEngine, scope: Scope, data: Uint8Array | undefined): boolean {
  if (!data) return false;
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(data));
  } catch {
    return false;
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return false;
  db.seed(scope, parsed as Record<string, unknown>);
  return !db.isEmpty(scope);
}
