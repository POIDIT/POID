/**
 * The wire protocol between a POID application (in the sandbox) and the host.
 *
 * This module is the single source of truth for message shapes and the
 * capability model. `@poid/host` imports these as **types only** (erased at
 * compile time), so nothing here inflates the host and the two sides can
 * never drift apart.
 */

/** Protocol marker + version, present on every message so the bridge can tell
 * POID traffic from unrelated `postMessage` noise. */
export const PROTOCOL = 1 as const;

/**
 * A capability is a namespace the host grants for this run. The SDK builds a
 * `window.poid` namespace **only** if its capability is present — an ungranted
 * `poid.net`/`poid.ai` is genuinely `undefined`, never a throwing stub, so
 * absence is observable (RUNTIME-API §0, §4, §5).
 */
export type Capability =
  | "app"
  | "db.kv"
  | "db.sql"
  | "db.docs"
  | "db.slots"
  | "files"
  | "net"
  | "ai"
  | "mcp"
  | "ui"
  | "ui.print"
  | "ui.clipboard"
  | "export";

/** Every method the broker will dispatch. The broker fails closed on anything
 * not in this set (SECURITY §4). Kept as a runtime array so the host can build
 * its allowlist from the same source the SDK calls through. */
export const METHODS = [
  "app.info",
  "app.setTitle",
  "app.setDirty",
  "db.kv.get",
  "db.kv.set",
  "db.kv.delete",
  "db.kv.list",
  "db.kv.clear",
  "db.sql.exec",
  "db.sql.begin",
  "db.sql.commit",
  "db.sql.rollback",
  "db.docs.insert",
  "db.docs.find",
  "db.docs.findOne",
  "db.docs.update",
  "db.docs.delete",
  "db.docs.count",
  "db.slots.list",
  "db.slots.current",
  "files.open",
  "files.save",
  "net.fetch",
  "ai.complete",
  "ai.models",
  "mcp.servers",
  "mcp.call",
  "ui.notify",
  "ui.confirm",
  "ui.setBadge",
  "ui.print",
  "ui.clipboard.write",
  "ui.clipboard.read",
  "export.data",
  "export.pdf",
] as const;

/** A method name the broker recognises. */
export type Method = (typeof METHODS)[number];

/** Which capability each method belongs to. The broker rejects a call whose
 * capability was not granted with `PERMISSION_DENIED` (RUNTIME-API §0). */
export const METHOD_CAPABILITY: Record<Method, Capability> = {
  "app.info": "app",
  "app.setTitle": "app",
  "app.setDirty": "app",
  "db.kv.get": "db.kv",
  "db.kv.set": "db.kv",
  "db.kv.delete": "db.kv",
  "db.kv.list": "db.kv",
  "db.kv.clear": "db.kv",
  "db.sql.exec": "db.sql",
  "db.sql.begin": "db.sql",
  "db.sql.commit": "db.sql",
  "db.sql.rollback": "db.sql",
  "db.docs.insert": "db.docs",
  "db.docs.find": "db.docs",
  "db.docs.findOne": "db.docs",
  "db.docs.update": "db.docs",
  "db.docs.delete": "db.docs",
  "db.docs.count": "db.docs",
  "db.slots.list": "db.slots",
  "db.slots.current": "db.slots",
  "files.open": "files",
  "files.save": "files",
  "net.fetch": "net",
  "ai.complete": "ai",
  "ai.models": "ai",
  "mcp.servers": "mcp",
  "mcp.call": "mcp",
  "ui.notify": "ui",
  "ui.confirm": "ui",
  "ui.setBadge": "ui",
  "ui.print": "ui.print",
  "ui.clipboard.write": "ui.clipboard",
  "ui.clipboard.read": "ui.clipboard",
  "export.data": "export",
  "export.pdf": "export",
};

/** Reader-supplied identity + platform facts (RUNTIME-API §1). */
export interface AppInfoDescriptor {
  id: string;
  name: string;
  version: string;
  instanceId: string;
  readerVersion: string;
  platform: "windows" | "macos" | "linux" | "ios" | "android" | "web";
  storageMode: "embedded" | "vault" | "connection";
  slot: string | null;
}

/** The synchronous bootstrap the host injects before app code runs. The SDK
 * reads it to decide which namespaces to build; the app cannot influence it. */
export interface BootstrapConfig {
  app: AppInfoDescriptor;
  capabilities: Capability[];
}

/** Application → host: a brokered method call. */
export interface RequestMessage {
  poid: typeof PROTOCOL;
  id: number;
  method: string;
  params: unknown;
}

/** Host → application: a successful result. */
export interface ResultMessage {
  poid: typeof PROTOCOL;
  id: number;
  ok: true;
  result: unknown;
}

/** Host → application: a failure, already scrubbed to a §9 error code. */
export interface FailureMessage {
  poid: typeof PROTOCOL;
  id: number;
  ok: false;
  error: { code: string; message: string };
}

/** Host → application: a lifecycle event push (e.g. `suspend`). */
export interface EventMessage {
  poid: typeof PROTOCOL;
  event: string;
  data?: unknown;
}

/** Host ↔ application liveness. The SDK auto-answers `ping` with `pong` from
 * its message handler, so an app stuck in an infinite loop stops answering and
 * the watchdog can detect it without the app's cooperation. */
export interface PingMessage {
  poid: typeof PROTOCOL;
  ping: number;
}
export interface PongMessage {
  poid: typeof PROTOCOL;
  pong: number;
}

/** Any message the host may send to the sandbox. */
export type HostMessage = ResultMessage | FailureMessage | EventMessage | PingMessage;
