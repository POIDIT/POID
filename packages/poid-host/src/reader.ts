/**
 * Browser-side reader wiring: consent, the sandboxed iframe, the bridge, the
 * broker and the watchdog composed into a running document window. This is the
 * code the Playwright tier exercises end to end; the pieces it wires are each
 * unit-tested in Node.
 *
 * The order is the security order: **consent first, execution second.** The
 * iframe that runs the application is not created until the user chooses Run,
 * so an application cannot suppress or race the consent screen — it does not
 * exist yet when consent is shown.
 */

import type { Capability } from "@poid/sdk";
import { type ChromeHandle, type ConsentManifest, renderChrome, renderConsent } from "@poid/ui";
import { SandboxBridge } from "./bridge.js";
import { Broker, type BrokerHandlers, type Connection, type ReaderSession } from "./broker.js";
import { ContainerServer } from "./container-server.js";
import { SANDBOX_TOKENS } from "./csp.js";
import { type DataEngine, InMemoryEngine } from "./engine.js";
import { Watchdog, type WatchdogOptions } from "./watchdog.js";

/** Everything needed to open one POID in a reader window. */
export interface MountOptions {
  /** Host element to render chrome and the iframe into. */
  container: HTMLElement;
  /** Container files (as opened by `poid-core`). */
  files: Map<string, Uint8Array>;
  /** Manifest `entry`. */
  entry: string;
  /** Consent-relevant manifest facts. */
  manifest: ConsentManifest & {
    instanceId: string;
    storageMode: "embedded" | "vault" | "connection";
  };
  /** Granted capabilities (already reconciled with user consent). */
  capabilities: Capability[];
  /** Approved network origins for CSP `connect-src`. */
  connectSrc?: string[];
  /** The `@poid/sdk` sandbox bootstrap, bundled to a self-executing IIFE. */
  sdkSource: string;
  /** Optional side-effecting handlers (net/ai/mcp/ui/files/export/sql). */
  handlers?: BrokerHandlers;
  /** User-configured connections (secrets held here, never exposed). */
  connections?: Map<string, Connection>;
  /** Storage backend; defaults to in-memory. */
  engine?: DataEngine;
  /** Per-POID quota in bytes; defaults to 64 MB (SPEC `storage.quota_mb`). */
  quotaBytes?: number;
  /** Watchdog tuning (short intervals in tests). */
  watchdog?: WatchdogOptions;
}

/** A live reader window handle. */
export interface ReaderHandle {
  /** Tears the application down: removes the iframe, stops the watchdog, and
   * unregisters the session. Always works, even if the app is hung. */
  stop(): void;
  /** The chrome handle, once the app is running. */
  chrome?: ChromeHandle;
}

let sessionCounter = 0;

/**
 * Mounts a POID. Shows consent; on Run, creates the sandboxed iframe and wires
 * the bridge; on Cancel, resolves without ever executing the application.
 */
export function mountReader(options: MountOptions): Promise<ReaderHandle> {
  return new Promise((resolve) => {
    const handle: ReaderHandle = { stop: () => {} };

    renderConsent(options.container, options.manifest, {
      onCancel: () => resolve(handle),
      onRun: () => resolve(run(options, handle)),
    });
  });
}

function run(options: MountOptions, handle: ReaderHandle): ReaderHandle {
  const doc = options.container.ownerDocument;
  const engine = options.engine ?? new InMemoryEngine();
  const broker = new Broker(engine, { handlers: options.handlers });
  const bridge = new SandboxBridge(broker);
  bridge.attach(
    doc.defaultView as unknown as {
      addEventListener(t: "message", l: (ev: MessageEvent) => void): void;
    },
  );

  const sessionId = `session-${++sessionCounter}`;
  const session: ReaderSession = {
    instanceId: options.manifest.instanceId,
    appInfo: {
      id: "app",
      name: options.manifest.name,
      version: options.manifest.version,
      readerVersion: "0.0.1",
      platform: "web",
      storageMode: options.manifest.storageMode,
    },
    capabilities: new Set(options.capabilities),
    slots: [],
    currentSlot: "",
    quotaBytes: options.quotaBytes ?? 64 * 1024 * 1024,
    connections: options.connections ?? new Map(),
  };
  broker.register(sessionId, session);

  // Chrome first (host DOM), then the sandboxed iframe.
  const chrome = renderChrome(
    options.container,
    { title: options.manifest.name, storageMode: options.manifest.storageMode },
    () => handle.stop(),
  );
  handle.chrome = chrome;

  const server = new ContainerServer({
    files: options.files,
    entry: options.entry,
    sdkSource: options.sdkSource,
    bootstrap: {
      app: {
        id: session.appInfo.id,
        name: session.appInfo.name,
        version: session.appInfo.version,
        instanceId: session.instanceId,
        readerVersion: session.appInfo.readerVersion,
        platform: "web",
        storageMode: session.appInfo.storageMode,
        slot: null,
      },
      capabilities: options.capabilities,
    },
    csp: { connectSrc: options.connectSrc },
  });
  const entryHtml = new TextDecoder().decode(server.resolve(server.entryPath).body);

  const iframe = doc.createElement("iframe");
  // The one line that must never regress: allow-scripts, never allow-same-origin.
  iframe.setAttribute("sandbox", SANDBOX_TOKENS);
  const blob = new Blob([entryHtml], { type: "text/html" });
  const blobUrl = URL.createObjectURL(blob);
  iframe.src = blobUrl;
  options.container.append(iframe);

  const win = iframe.contentWindow;
  const watchdog = new Watchdog(
    (seq) => win?.postMessage({ poid: 1, ping: seq }, "*"),
    () => chrome.setUnresponsive(true),
    options.watchdog,
  );

  bridge.register({
    sessionId,
    sourceWindow: win,
    post: (message) => win?.postMessage(message, "*"),
    watchdog,
  });

  // Start immediately rather than on `load`: an app that blocks its event loop
  // during a synchronous inline script never fires `load`, and that is exactly
  // the case the watchdog must still catch. The generous timeout absorbs normal
  // load time.
  watchdog.start();

  handle.stop = () => {
    watchdog.stop();
    bridge.unregister(sessionId);
    broker.unregister(sessionId);
    iframe.remove();
    URL.revokeObjectURL(blobUrl);
  };

  return handle;
}
