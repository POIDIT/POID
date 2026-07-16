/**
 * The Web Reader shell: drop a `.poid` onto the page and it runs — entirely
 * in this browser. Wires the drop zone, the file picker and the PWA launch
 * queue to the open flow, and renders the outcome screens. No code here
 * touches the network; the only fetches this site ever makes are its own
 * static assets during page load.
 */

import { mountBanner } from "./banner.js";
import { triggerDownload, updatedFileBytes } from "./download.js";
import {
  type NoticeOutcome,
  openBytes,
  type RejectedOutcome,
  type RunnableOutcome,
  runContainer,
} from "./open-flow.js";
import { loadPoidWasm } from "./wasm-api.js";

// Injected at bundle time: the @poid/sdk sandbox bootstrap as a minified IIFE,
// exactly as the desktop reader embeds it (see scripts/build-site.mjs).
declare const __SDK_SOURCE__: string;

/** The PWA File Handling API surface (Chromium only). */
interface LaunchParams {
  files: { getFile(): Promise<File> }[];
}
interface LaunchQueue {
  setConsumer(consumer: (params: LaunchParams) => void): void;
}

interface ActiveSession {
  stop(): void;
}

function byId(id: string): HTMLElement {
  const node = document.getElementById(id);
  if (!node) throw new Error(`web reader shell is missing #${id}`);
  return node;
}

function show(section: HTMLElement): void {
  for (const id of ["home", "outcome", "reader"]) byId(id).hidden = true;
  section.hidden = false;
}

let active: ActiveSession | undefined;

function closeActive(): void {
  active?.stop();
  active = undefined;
}

function renderRejected(outcome: RejectedOutcome): void {
  const panel = byId("outcome");
  panel.replaceChildren();
  panel.className = "poid-outcome poid-outcome--error";

  const heading = document.createElement("h2");
  heading.textContent = "This file was refused";
  const codes = document.createElement("p");
  codes.className = "poid-outcome__code";
  codes.textContent = outcome.registry ? `${outcome.registry} · ${outcome.code}` : outcome.code;
  const explanation = document.createElement("p");
  explanation.textContent = outcome.explanation;
  const detail = document.createElement("p");
  detail.className = "poid-outcome__detail";
  detail.textContent = outcome.message;
  const reassurance = document.createElement("p");
  reassurance.className = "poid-outcome__safe";
  reassurance.textContent = "Nothing from this file was executed.";

  panel.append(heading, codes, explanation, detail, reassurance, backButton());
  show(panel);
}

function renderNotice(outcome: NoticeOutcome): void {
  const panel = byId("outcome");
  panel.replaceChildren();
  panel.className = "poid-outcome";

  const heading = document.createElement("h2");
  const body = document.createElement("p");
  if (outcome.kind === "data-container") {
    const ref = outcome.facts.dataRef;
    heading.textContent = "This is a data container";
    body.textContent =
      `It carries data for “${ref?.appId ?? "an application"}” ` +
      `(schema ${ref?.schema ?? "unknown"}, app version ${ref?.appVersion ?? "unknown"}) — no code, nothing to run. ` +
      "Open it with the matching application in POID Studio to import it.";
  } else if (outcome.kind === "workspace") {
    heading.textContent = "This is a workspace";
    body.textContent =
      "Workspaces bundle several applications. The web reader can't open them yet — POID Studio can.";
  } else {
    heading.textContent = "This application needs an engine the web reader doesn't have yet";
    const engines = outcome.missingEngines.join(", ") || "an additional language engine";
    body.textContent =
      `“${outcome.facts.name}” needs: ${engines}. ` +
      "The web reader currently runs JavaScript/WASM applications only. POID Studio provides additional engines.";
  }
  panel.append(heading, body, backButton());
  show(panel);
}

function backButton(): HTMLButtonElement {
  const back = document.createElement("button");
  back.type = "button";
  back.className = "poid-outcome__back";
  back.textContent = "Open another file";
  back.addEventListener("click", () => {
    closeActive();
    show(byId("home"));
  });
  return back;
}

async function renderRunnable(outcome: RunnableOutcome, filename: string): Promise<void> {
  const reader = byId("reader");
  const host = byId("reader-host");
  const toolbar = byId("reader-toolbar");
  host.replaceChildren();
  toolbar.replaceChildren();
  show(reader);

  const title = document.createElement("span");
  title.className = "poid-toolbar__file";
  title.textContent = filename;
  toolbar.append(title);

  const close = document.createElement("button");
  close.type = "button";
  close.className = "poid-toolbar__close";
  close.textContent = "Close";
  close.addEventListener("click", () => {
    closeActive();
    show(byId("home"));
  });

  const session = await runContainer(outcome, host, __SDK_SOURCE__);
  if (!session.ran) {
    outcome.poid.free();
    show(byId("home"));
    return;
  }

  active = {
    stop: () => {
      session.handle.stop();
      outcome.poid.free();
    },
  };

  if (outcome.facts.storageMode === "embedded") {
    const download = document.createElement("button");
    download.type = "button";
    download.className = "poid-toolbar__download";
    download.textContent = "Download updated file";
    download.addEventListener("click", () => {
      void session.engine.flush().then(() => {
        const bytes = updatedFileBytes(outcome.poid, session.engine.snapshot(session.scope));
        triggerDownload(document, bytes, filename);
      });
    });
    toolbar.append(download);
  }
  toolbar.append(close);
}

async function openFile(file: File): Promise<void> {
  closeActive();
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const outcome = await openBytes(bytes);
    if (outcome.kind === "rejected") {
      renderRejected(outcome);
    } else if (outcome.kind === "runnable") {
      await renderRunnable(outcome, file.name);
    } else {
      renderNotice(outcome);
    }
  } catch (e) {
    renderRejected({
      kind: "rejected",
      registry: undefined,
      code: "internal",
      message: e instanceof Error ? e.message : String(e),
      explanation: "Something went wrong while opening this file.",
    });
  }
}

function wireHome(): void {
  const zone = byId("drop-zone");
  const input = byId("file-input") as HTMLInputElement;

  byId("pick-button").addEventListener("click", () => input.click());
  input.addEventListener("change", () => {
    const file = input.files?.[0];
    if (file) void openFile(file);
    input.value = "";
  });

  for (const ev of ["dragover", "dragenter"] as const) {
    zone.addEventListener(ev, (e) => {
      e.preventDefault();
      zone.classList.add("poid-drop--over");
    });
  }
  zone.addEventListener("dragleave", () => zone.classList.remove("poid-drop--over"));
  zone.addEventListener("drop", (e) => {
    e.preventDefault();
    zone.classList.remove("poid-drop--over");
    const file = e.dataTransfer?.files[0];
    if (file) void openFile(file);
  });
}

function wireLaunchQueue(): void {
  const queue = (window as { launchQueue?: LaunchQueue }).launchQueue;
  queue?.setConsumer((params) => {
    const first = params.files[0];
    if (first) void first.getFile().then((file) => openFile(file));
  });
}

function registerServiceWorker(): void {
  if ("serviceWorker" in navigator && location.protocol.startsWith("http")) {
    navigator.serviceWorker.register("./sw.js").catch((e) => {
      console.warn("poid-web: service worker registration failed:", e);
    });
  }
}

function init(): void {
  wireHome();
  wireLaunchQueue();
  registerServiceWorker();
  mountBanner(document.body);
  // Preload the validation core so it is part of the initial page load —
  // after this, opening and running a POID makes zero network requests.
  void loadPoidWasm().catch((e) => console.warn("poid-web: wasm preload failed:", e));
}

init();
