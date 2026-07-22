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
import { Broker, type BrokerHandlers, type Diagnostic, type ReaderSession } from "./broker.js";
import { ContainerServer } from "./container-server.js";
import { SANDBOX_TOKENS } from "./csp.js";
import { type DataEngine, InMemoryEngine } from "./engine.js";
import { BlobOrigin, type SyntheticOrigin } from "./origin.js";
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
  /**
   * Where the real reason for a refusal goes. The application receives only a
   * §9 code, so without this the detail is discarded — route it to the log the
   * *user* can read (SPEC §7.2.4).
   */
  onDiagnostic?: (entry: Diagnostic) => void;
  /**
   * Runs after consent and before the application exists.
   *
   * This is where the Connection binding happens (SPEC §7.2.3): the reader
   * cannot know which backend serves the document until the user has consented
   * to run it at all, and must know before the first storage call. Whatever it
   * returns is merged over the static options — so the binding decides *which
   * handlers get installed*, which is how the reader expresses a binding
   * without ever telling the broker (and therefore without the application
   * being able to learn it).
   *
   * Returning an empty object leaves the static options untouched.
   */
  prepare?: () => Promise<PreparedMount>;
  /** Storage backend; defaults to in-memory. */
  engine?: DataEngine;
  /** Per-POID quota in bytes; defaults to 64 MB (SPEC `storage.quota_mb`). */
  quotaBytes?: number;
  /** The active slot and the slot list (SPEC §6.4); defaults to a single
   * anonymous slot. The application never chooses these — the reader does. */
  currentSlot?: string;
  slotNames?: string[];
  /** The synthetic origin that serves the app + subresources (SPEC §5.2.1).
   * Defaults to {@link BlobOrigin} — one document, no subresource serving. */
  origin?: SyntheticOrigin;
  /** Watchdog tuning (short intervals in tests). */
  watchdog?: WatchdogOptions;
}

/** What {@link MountOptions.prepare} may decide once consent is given. */
export interface PreparedMount {
  /** Handlers to use instead of the static ones (the bound backend). */
  handlers?: BrokerHandlers;
  /**
   * What the chrome's storage badge should say.
   *
   * The manifest's `storage.mode` is what the application *asked for*; after
   * binding, this is what it actually got. A POID that declared `connection`
   * and whose user kept the data local must not display "connection" — the
   * badge is the user's own record of where their data went.
   */
  storageBadge?: string;
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

async function run(options: MountOptions, handle: ReaderHandle): Promise<ReaderHandle> {
  const doc = options.container.ownerDocument;

  // Consent has been given; the application still does not exist. This is the
  // only window in which the binding question can be asked (SPEC §7.2.3):
  // after the user agreed to run this program at all, before anything of it
  // has been loaded that could interfere with the asking.
  const prepared = (await options.prepare?.()) ?? {};

  const engine = options.engine ?? new InMemoryEngine();
  const broker = new Broker(engine, {
    handlers: prepared.handlers ?? options.handlers,
    onDiagnostic: options.onDiagnostic,
  });
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
    slots: options.slotNames ?? [],
    currentSlot: options.currentSlot ?? "",
    quotaBytes: options.quotaBytes ?? 64 * 1024 * 1024,
  };
  broker.register(sessionId, session);

  // Chrome first (host DOM), then the sandboxed iframe.
  const chrome = renderChrome(
    options.container,
    {
      title: options.manifest.name,
      // What the data placement actually resolved to, not what the manifest
      // requested — see PreparedMount.storageBadge.
      storageMode: prepared.storageBadge ?? options.manifest.storageMode,
    },
    () => handle.stop(),
  );
  handle.chrome = chrome;

  // The synthetic origin (SPEC §5.2.1) serves the app + subresources so a
  // multi-file app's `<script src>` executes; its CSP asset-source is baked
  // into the entry before serving. The blob fallback serves only the entry
  // (single-file parity, inline-only).
  const origin = options.origin ?? new BlobOrigin(doc.defaultView ?? globalThis);

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
    csp: { connectSrc: options.connectSrc, assetSource: origin.cspAssetSource() },
  });
  const src = await origin.serve(sessionId, server.assets(), server.entryPath);

  const iframe = doc.createElement("iframe");
  // The one line that must never regress: allow-scripts, never allow-same-origin.
  iframe.setAttribute("sandbox", SANDBOX_TOKENS);
  iframe.src = src;
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
    origin.revoke(sessionId);
  };

  return handle;
}
