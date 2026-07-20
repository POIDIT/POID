/**
 * The Reader window: a document window and nothing else (ARCHITECTURE §2).
 *
 * Asks the Rust core for its document (`reader_bootstrap` — scope derived
 * from this window's label), routes it, and either mounts the sandboxed
 * application (`@poid/host` — the same consent, sandbox, broker and watchdog
 * as the Web Reader) or renders an honest outcome panel. No code here can
 * reach the hub; there is nothing hub-like in this bundle at all.
 */

import {
  capabilitiesFromGrant,
  consentManifestFrom,
  hostFacts,
  IndexedDbEngine,
  mountReader,
  runGrant,
  type Scope,
  seedFromStore,
} from "@poid/host";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  type DesktopOutcome,
  type NoticeOutcome,
  type RejectedOutcome,
  type RunnableOutcome,
  routeDocument,
} from "./desktop-flow.js";
import type { DocumentDto } from "./document-dto.js";

// Injected at bundle time: the @poid/sdk sandbox bootstrap as a minified
// IIFE (see scripts/build-ui.mjs — the same pattern as the Web Reader).
declare const __SDK_SOURCE__: string;

/** The desktop vault database name (separate from the Web Reader's). */
const VAULT_DB = "poid-studio-vault";

function byId(id: string): HTMLElement {
  const node = document.getElementById(id);
  if (!node) throw new Error(`reader window is missing #${id}`);
  return node;
}

function el(tag: string, className?: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function outcomePanel(error: boolean): { main: HTMLElement; panel: HTMLElement } {
  const main = el("main");
  const panel = el("section", `poid-outcome${error ? " poid-outcome--error" : ""}`);
  main.append(panel);
  return { main, panel };
}

function renderRejected(root: HTMLElement, outcome: RejectedOutcome): void {
  const { main, panel } = outcomePanel(true);
  panel.append(
    el("p", "poid-outcome__code", outcome.registry ?? outcome.code),
    el("h2", undefined, "This file was refused"),
    el("p", undefined, outcome.explanation),
    el("p", "poid-outcome__safe", "Nothing in it was run."),
    el("p", "poid-outcome__detail", `${outcome.fileName} — ${outcome.message}`),
  );
  root.replaceChildren(main);
}

function renderNotice(root: HTMLElement, outcome: NoticeOutcome): void {
  const { main, panel } = outcomePanel(false);
  if (outcome.kind === "data-container") {
    panel.append(
      el("h2", undefined, "This is a data file, not an application"),
      el(
        "p",
        undefined,
        `It holds data for “${outcome.facts.name}” (v${outcome.facts.dataRef?.appVersion ?? outcome.facts.version}). Open it with that application.`,
      ),
    );
  } else if (outcome.kind === "workspace") {
    panel.append(
      el("h2", undefined, "This is a workspace"),
      el(
        "p",
        undefined,
        "Workspaces bundle several applications. Opening workspaces is not part of this version of POID Studio yet.",
      ),
    );
  } else {
    panel.append(
      el("h2", undefined, "This application needs an engine this reader does not have yet"),
      el(
        "p",
        undefined,
        `It asks for: ${outcome.missingEngines.join(", ")}. Engine support in the desktop reader is coming; the file itself is fine.`,
      ),
    );
  }
  panel.append(el("p", "poid-outcome__detail", outcome.fileName));
  root.replaceChildren(main);
}

async function runOutcome(root: HTMLElement, outcome: RunnableOutcome): Promise<void> {
  const { facts, files } = outcome;
  if (!facts.instanceId || !facts.entry) {
    // Unreachable: routeDocument assigns the id and poid-core validated `entry`.
    throw new Error("runnable container without instance id or entry");
  }

  const engine = await IndexedDbEngine.open(VAULT_DB);
  const scope: Scope = { instanceId: facts.instanceId, slot: "" };
  await engine.hydrate(scope);

  let seeded = false;
  if (facts.storageMode === "embedded" && !facts.protectedData && engine.isEmpty(scope)) {
    seeded = seedFromStore(engine, scope, files.get("data/store.json"));
  }

  const handle = await mountReader({
    container: root,
    files,
    entry: facts.entry,
    manifest: {
      ...consentManifestFrom(facts, outcome.signature),
      instanceId: facts.instanceId,
      storageMode: facts.storageMode,
    },
    capabilities: capabilitiesFromGrant(hostFacts(facts), runGrant(facts)),
    connectSrc: facts.permissions.network,
    sdkSource: __SDK_SOURCE__,
    engine,
  });

  if (handle.chrome === undefined) {
    // Consent declined: roll back a fresh seed and close the document window
    // — a declined app leaves no trace and no empty shell.
    if (seeded) {
      engine.kvClear(scope);
      await engine.flush();
    }
    await getCurrentWindow().close();
  }
}

async function main(): Promise<void> {
  const root = byId("reader-root");
  let dto: DocumentDto;
  try {
    dto = await invoke<DocumentDto>("reader_bootstrap");
  } catch (e) {
    const { main: m, panel } = outcomePanel(true);
    panel.append(
      el("h2", undefined, "This window lost its document"),
      el("p", undefined, "Close it and open the file again."),
      el("p", "poid-outcome__detail", String(e)),
    );
    root.replaceChildren(m);
    return;
  }

  const outcome: DesktopOutcome = routeDocument(dto);
  if (outcome.kind === "rejected") {
    renderRejected(root, outcome);
  } else if (outcome.kind === "runnable") {
    await runOutcome(root, outcome);
  } else {
    renderNotice(root, outcome);
  }
}

void main();
