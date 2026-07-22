/**
 * `@poid/host` — the host side of the sandbox bridge and the boundary broker.
 *
 * This package owns the security boundary between a POID application and the
 * reader: it creates the sandboxed iframe, serves the container from a
 * dedicated synthetic origin (SPEC §5.2.1), applies the CSP, and brokers every
 * `window.poid` call with scope derived from the sending window. Read
 * `spec/SECURITY.md` before changing it.
 */

export { type IncomingMessage, SandboxBridge } from "./bridge.js";
export {
  Broker,
  BrokerError,
  type BrokerHandlers,
  type Diagnostic,
  type ReaderSession,
} from "./broker.js";
export { capabilitiesFromGrant, type Grant, type ManifestFacts } from "./capabilities.js";
export {
  connectionScope,
  type LoopbackConnectionOptions,
  loopbackConnection,
} from "./connection.js";
export {
  ContainerServer,
  type ContainerServerInput,
  injectRuntime,
  type ServedResponse,
} from "./container-server.js";
export { buildCsp, type CspOptions, SANDBOX_TOKENS } from "./csp.js";
export { compileQuery, compileSort, fieldPath } from "./docs-query.js";
export {
  DocsStore,
  docsBrokerHandler,
  docsRegexpFunction,
  type FindOptions,
} from "./docs-store.js";
export { type DataEngine, InMemoryEngine, type Scope } from "./engine.js";
export {
  EngineIntegrityError,
  type EngineManifest,
  engineSatisfies,
  verifyEngine,
} from "./engines.js";
export { explain, FALLBACK_EXPLANATION, REGISTRY_EXPLANATIONS } from "./explanations.js";
export { type GuardResult, guardRequest, SCOPE_FIELDS } from "./guard.js";
export { IndexedDbEngine } from "./idb-engine.js";
export {
  BUILTIN_PROFILE_ENGINES,
  consentManifestFrom,
  extractFacts,
  hostFacts,
  type ReaderManifestFacts,
  runGrant,
  unsupportedProfileEngines,
} from "./manifest-facts.js";
export {
  applyMigrations,
  type Migration,
  parseMigrations,
  schemaVersionOf,
} from "./migrations.js";
export { BlobOrigin, type SyntheticOrigin } from "./origin.js";
export {
  type MountOptions,
  mountReader,
  type PreparedMount,
  type ReaderHandle,
} from "./reader.js";
export { seedFromStore } from "./seed.js";
export { dumpSql } from "./sql-dump.js";
export {
  type ScalarFunction,
  type ScopeResolver,
  type SqlCallOptions,
  type SqlEngineOptions,
  type SqlResult,
  sessionScope,
  sqlBrokerHandler,
  WaSqliteEngine,
} from "./sql-engine.js";
export {
  IdbSqlPersistence,
  makeSqlHandlers,
  type SqlHandlers,
  type SqlHandlersOptions,
  type SqlPersistence,
} from "./sql-persistence.js";
export type { SqliteWasmSource } from "./sql-wasm-api.js";
export { Watchdog, type WatchdogClock, type WatchdogOptions } from "./watchdog.js";
