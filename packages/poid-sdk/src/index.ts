/**
 * `@poid/sdk` — the API injected into the sandboxed context as `window.poid`.
 *
 * Zero runtime dependencies; must stay under 10 KB minified (a size test
 * enforces it). The host injects this module as the first script in the
 * sandboxed document and calls {@link installPoid} synchronously, so
 * `window.poid` exists before any application code runs.
 */

import { buildPoidApi, type PoidAPI } from "./api.js";
import { type BootstrapConfig, PROTOCOL } from "./protocol.js";
import { RpcClient, type Transport } from "./rpc.js";

export type { PoidAPI } from "./api.js";
export type { PoidErrorCode } from "./errors.js";
export { ERROR_CODES, PoidError } from "./errors.js";
export type {
  AppInfoDescriptor,
  BootstrapConfig,
  Capability,
  EventMessage,
  FailureMessage,
  HostMessage,
  Method,
  PingMessage,
  PongMessage,
  RequestMessage,
  ResultMessage,
} from "./protocol.js";
export { METHOD_CAPABILITY, METHODS, PROTOCOL } from "./protocol.js";
export type { Transport } from "./rpc.js";
export { RpcClient } from "./rpc.js";

/** Builds a `poid` API object over any transport. The browser bootstrap uses
 * a `postMessage` transport; tests inject their own. */
export function createPoid(config: BootstrapConfig, transport: Transport): PoidAPI {
  return buildPoidApi(config, new RpcClient(transport));
}

/** The global the host embeds before this script; read once, then removed. */
interface BootstrapGlobal {
  __POID_BOOTSTRAP__?: BootstrapConfig;
}

/**
 * Installs `window.poid` inside the sandbox. Reads the bootstrap the host
 * injected, wires a `postMessage` transport to the parent window, and freezes
 * the API so application code cannot swap namespaces back in. Returns the API,
 * or `undefined` if there is no bootstrap (not running inside a POID host).
 */
export function installPoid(scope: Window & BootstrapGlobal = window): PoidAPI | undefined {
  const config = scope.__POID_BOOTSTRAP__;
  if (!config) return undefined;
  // Remove the bootstrap so application code cannot read or re-run it.
  delete scope.__POID_BOOTSTRAP__;

  const transport: Transport = {
    post: (message) => scope.parent.postMessage(message, "*"),
    onMessage: (handler) => {
      scope.addEventListener("message", (ev: MessageEvent) => {
        // Only the host (our parent) may drive the bridge.
        if (ev.source !== scope.parent) return;
        const data = ev.data as { poid?: unknown };
        if (!data || data.poid !== PROTOCOL) return;
        handler(ev.data);
      });
    },
  };

  const api = deepFreeze(buildPoidApi(config, new RpcClient(transport)));
  Object.defineProperty(scope, "poid", { value: api, writable: false, configurable: false });
  return api;
}

/** Freezes an object graph one level deep past the namespaces, enough to keep
 * application code from monkey-patching the surface. */
function deepFreeze<T>(obj: T): T {
  for (const key of Object.keys(obj as object)) {
    const value = (obj as Record<string, unknown>)[key];
    if (value && typeof value === "object") Object.freeze(value);
  }
  return Object.freeze(obj);
}
