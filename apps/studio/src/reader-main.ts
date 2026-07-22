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
  type BrokerHandlers,
  capabilitiesFromGrant,
  consentManifestFrom,
  type DataEngine,
  hostFacts,
  IdbSqlPersistence,
  IndexedDbEngine,
  loopbackConnection,
  makeSqlHandlers,
  mountReader,
  parseMigrations,
  runGrant,
  type Scope,
  type SqlHandlers,
  type SqlPersistence,
  seedFromStore,
} from "@poid/host";
import { type BindingChoice, renderBindingChoice } from "@poid/ui";
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
import { VaultSqlPersistence } from "./sql-ipc.js";
import { TauriOrigin } from "./tauri-origin.js";
import { switchSlot, VaultIpcEngine } from "./vault-ipc-engine.js";

// Injected at bundle time: the @poid/sdk sandbox bootstrap as a minified
// IIFE (see scripts/build-ui.mjs — the same pattern as the Web Reader).
declare const __SDK_SOURCE__: string;

/** The desktop vault database name (separate from the Web Reader's). */
const VAULT_DB = "poid-studio-vault";

/** IndexedDB name for embedded-mode SQL bytes (vault mode uses Rust IPC). */
const SQL_DB = "poid-studio-sql";

/** The `poid://` synthetic origin (SPEC §5.2.1), created once per window. */
let syntheticOrigin: Promise<TauriOrigin> | undefined;
function origin(): Promise<TauriOrigin> {
  syntheticOrigin ??= TauriOrigin.create();
  return syntheticOrigin;
}

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

/** The Fork / Move / Share prompt (SPEC §6.3). Honest by design: no system
 * can tell a copy from its original by content, so we ask — once. */
function renderCopyPrompt(root: HTMLElement, existingPath: string): Promise<string> {
  return new Promise((resolve) => {
    const { main, panel } = outcomePanel(false);
    panel.append(
      el("h2", undefined, "This file looks like a copy"),
      el(
        "p",
        undefined,
        "It carries the same identity as a file that still exists — a copied file is byte-identical to its original, so no reader can tell them apart. What should this one be?",
      ),
      el("p", "poid-outcome__detail", `The other file: ${existingPath}`),
    );
    const actions = el("div", "poid-consent__actions");
    const make = (label: string, choice: string, primary: boolean): HTMLElement => {
      const button = el("button", primary ? "poid-consent__run" : undefined, label);
      button.addEventListener("click", () => resolve(choice));
      return button;
    };
    actions.append(
      make("Share memory", "share", false),
      make("This was a move", "move", false),
      make("Fork (own memory)", "fork", true),
    );
    panel.append(actions);
    root.replaceChildren(main);
  });
}

/**
 * `poid.net.fetch`, forwarded to Core (SPEC §7.2.5).
 *
 * Nothing is decided here. This window checks nothing, holds no credential and
 * knows no allowlist: it hands the request across IPC, and Rust does the
 * allowlist check, the DNS resolution, the address validation, the header
 * stripping and the credential injection. Keeping the decisions on the far
 * side is what makes them true regardless of what happens in a web context.
 */
const netHandler: NonNullable<BrokerHandlers["net"]> = async (_session, params) => {
  const init = (params.init ?? {}) as {
    method?: string;
    headers?: Record<string, string>;
    body?: string;
  };
  return invoke("net_fetch", {
    url: String(params.url),
    method: init.method ?? "GET",
    headers: Object.entries(init.headers ?? {}),
    body: init.body ?? null,
  });
};

/** What the Rust side reports for the binding prompt (SPEC §7.2.3). */
interface BindingOptions {
  candidates: {
    id: string;
    kind: string;
    label: string;
    target: string;
    hasSecret: boolean;
  }[];
  /** `"unset"`, `"local"`, or a connection id. */
  recorded: string;
}

/**
 * Resolves where a connection-mode POID keeps its data.
 *
 * Asks only when there is no recorded answer: `"unset"` means the user has
 * never been asked, while `"local"` means they were asked and declined. Those
 * are different, and conflating them turns a consent prompt into nagware that
 * people learn to click through — which is how consent stops meaning anything.
 */
async function resolveBinding(
  root: HTMLElement,
  facts: RunnableOutcome["facts"],
): Promise<BindingChoice> {
  const require = facts.requires?.kind ?? "sql";

  const ask = async (): Promise<BindingChoice> => {
    const options = await invoke<BindingOptions>("connection_choices", {
      require,
      hint: facts.requires?.hint ?? null,
    });

    if (options.recorded === "local") return { kind: "local" };
    if (options.recorded !== "unset") {
      // A recorded connection still has to exist and still be usable; the
      // world moves between launches.
      const still = options.candidates.find((c) => c.id === options.recorded && c.hasSecret);
      if (still) return { kind: "connection", id: still.id };
      // It does not — fall through and ask again rather than failing at the
      // first query with a message the user cannot act on.
    }

    return new Promise<BindingChoice>((resolve) => {
      renderBindingChoice(
        root,
        {
          appName: facts.name,
          need: require,
          canConnect: true,
          candidates: options.candidates,
        },
        {
          onChoose: resolve,
          onConfigure: () => void invoke("open_hub"),
          onRefresh: () => void ask().then(resolve),
        },
      );
    });
  };

  const choice = await ask();
  // Record it, so the user is asked once rather than at every launch.
  try {
    await invoke("connection_bind", {
      require,
      choice: choice.kind === "local" ? "local" : choice.id,
    });
  } catch {
    // A binding we could not persist is still a binding for this session: the
    // user answered, and re-prompting them now would be worse than forgetting
    // the answer at the next launch.
  }
  return choice;
}

/** A thin slot bar (SPEC §6.4): switching is a reader operation; the app is
 * remounted into the new scope and never observes the switch. */
function renderSlotBar(
  root: HTMLElement,
  engine: VaultIpcEngine,
  onSwitch: (slot: string) => void,
): HTMLElement {
  const bar = el("div", "reader-slots");
  bar.append(el("span", "reader-slots__label", "Slot:"));
  const select = document.createElement("select");
  const names = engine.slots.length > 0 ? engine.slots : [engine.slot];
  for (const name of names) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name === "" ? "(default)" : name;
    option.selected = name === engine.slot;
    select.append(option);
  }
  const fresh = document.createElement("option");
  fresh.value = " new";
  fresh.textContent = "+ new slot…";
  select.append(fresh);
  select.addEventListener("change", () => {
    if (select.value === " new") {
      const name = window.prompt("Name for the new slot:");
      if (name?.trim()) onSwitch(name.trim());
      else select.value = engine.slot;
    } else if (select.value !== engine.slot) {
      onSwitch(select.value);
    }
  });
  bar.append(select);
  root.before(bar);
  return bar;
}

async function runOutcome(root: HTMLElement, outcome: RunnableOutcome): Promise<void> {
  const { facts, files } = outcome;
  if (!facts.instanceId || !facts.entry) {
    // Unreachable: routeDocument assigns the id and poid-core validated `entry`.
    throw new Error("runnable container without instance id or entry");
  }

  let engine: DataEngine & { flush(): Promise<void> };
  let scope: Scope;
  let seeded = false;
  let slotBar: HTMLElement | undefined;

  // The SQL tier (M10.3): vault mode persists to the Rust vault over IPC,
  // embedded mode to IndexedDB — the same homes as the kv tier. The engine
  // itself wakes lazily on the first poid.db.sql / poid.db.docs call, and an
  // embedded container's `data/database.sql` seeds a fresh scope then.
  // Connection mode (M10.5) is served by a loopback connection keyed by the
  // app id — data shared across files, not carried in the container.
  const sqlWasm = { wasmUrl: "wa-sqlite.wasm" };
  const sqlPersistence: SqlPersistence =
    facts.storageMode === "vault"
      ? new VaultSqlPersistence()
      : await IdbSqlPersistence.open(SQL_DB);

  /** The local SQL tier: the same store `embedded`/`vault` use. */
  const localSql = (): SqlHandlers =>
    makeSqlHandlers({
      wasm: sqlWasm,
      persistence: sqlPersistence,
      seedSql:
        facts.storageMode === "embedded" && !facts.protectedData
          ? files.get("data/database.sql")
          : undefined,
      // Update-in-place migrations (SPEC §12).
      schemaVersion: facts.schemaVersion,
      migrations: parseMigrations(files),
    });

  // In connection mode the handlers depend on a decision the user has not made
  // yet — the binding prompt runs after consent (SPEC §7.2.3) — so `prepare`
  // below may replace this. Building it now costs nothing: makeSqlHandlers is
  // lazy and creates no engine until the first query.
  let sqlHandlers: SqlHandlers = localSql();

  if (facts.storageMode === "vault") {
    const vaultEngine = await VaultIpcEngine.open();
    engine = vaultEngine;
    scope = { instanceId: facts.instanceId, slot: vaultEngine.slot };
    if (facts.slots) {
      slotBar = renderSlotBar(root, vaultEngine, (slot) => {
        void (async () => {
          await vaultEngine.flush();
          // Drain SQL write-behind before the switch: the Rust side saves
          // under the *current* slot, which is about to change.
          await sqlHandlers.flush();
          await switchSlot(slot);
          // Remount from scratch: the application must start over in the new
          // scope, never observe the switch (SPEC §6.4).
          window.location.reload();
        })();
      });
    }
  } else {
    const idb = await IndexedDbEngine.open(VAULT_DB);
    scope = { instanceId: facts.instanceId, slot: "" };
    await idb.hydrate(scope);
    if (facts.storageMode === "embedded" && !facts.protectedData && idb.isEmpty(scope)) {
      seeded = seedFromStore(idb, scope, files.get("data/store.json"));
    }
    engine = idb;
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
    handlers: { sql: sqlHandlers.sql, docs: sqlHandlers.docs, net: netHandler },
    // After consent, before the application exists (SPEC §7.2.3).
    prepare:
      facts.storageMode === "connection"
        ? async () => {
            const choice = await resolveBinding(root, facts);
            if (choice.kind === "local") {
              // The user declined every connection. The application runs
              // unchanged and keeps its data here; the badge says so, because
              // the badge is the user's record of where their data went.
              return { storageBadge: "local (not connected)" };
            }
            // Keyed by the *connection*, not by the app: every POID bound to
            // one connection shares its database, which is the defining
            // property of a connection — data lives with the backend, not with
            // the file.
            sqlHandlers = loopbackConnection({
              connectionId: choice.id,
              wasm: sqlWasm,
              persistence: sqlPersistence,
            });
            return {
              handlers: { sql: sqlHandlers.sql, docs: sqlHandlers.docs, net: netHandler },
              storageBadge: "connection",
            };
          }
        : undefined,
    quotaBytes: facts.quotaMb === null ? undefined : facts.quotaMb * 1024 * 1024,
    currentSlot: scope.slot,
    slotNames: engine instanceof VaultIpcEngine ? engine.slots : [],
    // The synthetic origin so multi-file apps' subresources load (SPEC §5.2.1).
    origin: await origin(),
  });

  if (handle.chrome === undefined) {
    // Consent declined: roll back a fresh seed and close the document window
    // — a declined app leaves no trace and no empty shell.
    slotBar?.remove();
    if (seeded && engine instanceof IndexedDbEngine) {
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

  try {
    // A genuine copy asks Fork / Move / Share before anything runs (§6.3).
    if (dto.kind === "loaded" && dto.copyConflict) {
      const choice = await renderCopyPrompt(root, dto.copyConflict.existingPath);
      dto = await invoke<DocumentDto>("resolve_copy_conflict", { choice });
      // Clear the prompt: the reader chrome appends into this container, so a
      // leftover prompt would sit on top of the consent screen.
      root.replaceChildren();
    }

    const outcome: DesktopOutcome = routeDocument(dto);
    if (outcome.kind === "rejected") {
      renderRejected(root, outcome);
    } else if (outcome.kind === "runnable") {
      await runOutcome(root, outcome);
    } else {
      renderNotice(root, outcome);
    }
  } catch (e) {
    const { main: m, panel } = outcomePanel(true);
    panel.append(
      el("h2", undefined, "This document could not be opened"),
      el("p", undefined, "Close the window and try again."),
      el("p", "poid-outcome__detail", String(e)),
    );
    root.replaceChildren(m);
  }
}

void main();
