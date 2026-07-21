/**
 * From dropped bytes to a decision: validated by the WASM core, then routed
 * to the screen the container calls for. Only `type: app` with the plain
 * `web` profile executes; everything else gets an honest explanation —
 * never a silent failure, never a degraded run (M05: display limitations,
 * don't hide them).
 */

import {
  capabilitiesFromGrant,
  consentManifestFrom,
  explain,
  extractFacts,
  hostFacts,
  IdbSqlPersistence,
  IndexedDbEngine,
  makeSqlHandlers,
  mountReader,
  type ReaderHandle,
  type ReaderManifestFacts,
  runGrant,
  type Scope,
  type SqlHandlers,
  seedFromStore,
} from "@poid/host";
import { VaultWasmEngine } from "./vault-engine.js";
import { isContainerError, loadPoidWasm, type WebPoidHandle } from "./wasm-api.js";
import { BlobRewriteOrigin } from "./web-origin.js";

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
  facts: ReaderManifestFacts;
  /** For `engine-missing`: the engines the manifest calls for. */
  missingEngines: string[];
}

/** A runnable `type: app`, `web` profile container. */
export interface RunnableOutcome {
  kind: "runnable";
  facts: ReaderManifestFacts;
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

/** The engines the Web Reader mounts with (both flush asynchronously). */
export type WebRunEngine = IndexedDbEngine | VaultWasmEngine;

/** IndexedDB database name for the Web Reader's SQL blobs (M10). */
const SQL_DB = "poid-web-sql";

/** A mounted (or consent-declined) reader session. */
export interface RunSession {
  handle: ReaderHandle;
  /** False when the user chose Cancel on the consent screen. */
  ran: boolean;
  engine: WebRunEngine;
  /** The SQL tier for this session; `flush`/`dump` feed the download path. */
  sql: SqlHandlers;
  scope: Scope;
}

/**
 * Shows consent and, on Run, mounts the reader (`@poid/host` — the same
 * consent, sandbox, broker and watchdog as the desktop reader).
 *
 * `storage.mode` picks the engine: `vault` mounts the CRDT engine (the same
 * Automerge document as the desktop vault, persisted in IndexedDB); anything
 * else uses the embedded engine, seeded from `data/store.json` *before*
 * consent — and rolled back if the user cancels, so a declined app leaves no
 * trace.
 */
export async function runContainer(
  outcome: RunnableOutcome,
  container: HTMLElement,
  sdkSource: string,
  engine?: WebRunEngine,
): Promise<RunSession> {
  const { facts, poid, files } = outcome;
  if (!facts.instanceId || !facts.entry) {
    // Unreachable: openBytes assigns the id and poid-core validated `entry`.
    throw new Error("runnable container without instance id or entry");
  }
  const scope: Scope = { instanceId: facts.instanceId, slot: "" };

  let db: WebRunEngine;
  let seeded = false;
  if (facts.storageMode === "vault") {
    // A vault file carries no data (SPEC §6.1) — nothing to seed.
    db = engine ?? (await VaultWasmEngine.open(await loadPoidWasm(), facts.instanceId));
  } else {
    const idb = engine instanceof IndexedDbEngine ? engine : await IndexedDbEngine.open();
    await idb.hydrate(scope);
    if (facts.storageMode === "embedded" && !facts.protectedData && idb.isEmpty(scope)) {
      seeded = seedFromStore(idb, scope, poid.data());
    }
    db = idb;
  }

  // The SQL tier (M10): always persisted to IndexedDB in the Web Reader (it
  // cannot write back to the file). The engine wakes lazily on the first
  // poid.db.sql / poid.db.docs call — after consent — and an embedded
  // container's `data/database.sql` seeds a fresh scope then. A declined app
  // never reaches that first call, so it leaves no SQL trace.
  const sqlSeed =
    facts.storageMode === "embedded" && !facts.protectedData ? poid.sqlData() : undefined;
  const sql = makeSqlHandlers({
    wasm: { wasmUrl: new URL("wasm/wa-sqlite.wasm", document.baseURI).href },
    persistence: await IdbSqlPersistence.open(SQL_DB),
    seedSql: sqlSeed ?? undefined,
  });

  // Serve the app + subresources as blob URLs (SPEC §5.2.1) so a multi-file
  // app's `<script src>` runs — works offline and from file://, unlike a
  // service worker (which cannot control the sandboxed opaque-origin iframe).
  const origin = new BlobRewriteOrigin();

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
    handlers: { sql: sql.sql, docs: sql.docs },
    quotaBytes: facts.quotaMb === null ? undefined : facts.quotaMb * 1024 * 1024,
    origin,
  });

  const ran = handle.chrome !== undefined;
  if (!ran && seeded && db instanceof IndexedDbEngine) {
    db.kvClear(scope);
    await db.flush();
  }
  return { handle, ran, engine: db, sql, scope };
}
