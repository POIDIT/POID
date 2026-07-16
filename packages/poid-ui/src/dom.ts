/**
 * DOM rendering for the reader chrome. Framework-free: plain elements so the
 * chrome has zero runtime dependencies and can be mounted in any host window.
 *
 * Everything here lives in the **host** document, never in the sandboxed
 * iframe. The application has no reference to these nodes and no channel that
 * can reach them — the consent screen and the Stop button cannot be defeated
 * from inside the container.
 */

import { type ConsentManifest, type ConsentModel, consentModel } from "./consent.js";

type El = HTMLElement;

function el(doc: Document, tag: string, className?: string, text?: string): El {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** Callbacks for the consent decision. */
export interface ConsentHandlers {
  onRun(): void;
  onCancel(): void;
  onViewSource?(): void;
}

/**
 * Renders the consent screen into a host container and returns a teardown
 * function. The dialog blocks until the user decides; nothing from the
 * container can call these handlers.
 */
export function renderConsent(
  container: El,
  manifest: ConsentManifest,
  handlers: ConsentHandlers,
): () => void {
  const doc = container.ownerDocument;
  const model: ConsentModel = consentModel(manifest);

  const root = el(doc, "div", "poid-consent");
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");

  const header = el(doc, "header", "poid-consent__header");
  header.append(
    el(doc, "h1", "poid-consent__name", manifest.name),
    el(doc, "span", "poid-consent__version", `v${manifest.version}`),
  );
  const by = manifest.author ? `by ${manifest.author}` : "Unknown author";
  const sig =
    manifest.signature === "valid"
      ? "✓ Verified publisher"
      : manifest.signature === "invalid"
        ? "⚠ Invalid signature"
        : "⚠ Unsigned publisher";
  header.append(
    el(doc, "span", "poid-consent__author", by),
    el(doc, "span", "poid-consent__sig", sig),
  );

  const requests = el(doc, "section", "poid-consent__requests");
  requests.append(el(doc, "h2", undefined, "This application requests:"));
  const reqList = el(doc, "ul");
  for (const item of model.requests) reqList.append(el(doc, "li", "poid-consent__yes", item));
  requests.append(reqList);

  const notRequests = el(doc, "section", "poid-consent__not-requests");
  notRequests.append(el(doc, "h2", undefined, "It does NOT request:"));
  const noList = el(doc, "ul");
  for (const item of model.notRequests) noList.append(el(doc, "li", "poid-consent__no", item));
  notRequests.append(noList);

  const actions = el(doc, "div", "poid-consent__actions");
  const teardown = () => root.remove();

  if (handlers.onViewSource) {
    const view = el(doc, "button", "poid-consent__view", "View source") as HTMLButtonElement;
    view.type = "button";
    view.addEventListener("click", () => handlers.onViewSource?.());
    actions.append(view);
  }
  const cancel = el(doc, "button", "poid-consent__cancel", "Cancel") as HTMLButtonElement;
  cancel.type = "button";
  cancel.addEventListener("click", () => {
    teardown();
    handlers.onCancel();
  });
  const run = el(doc, "button", "poid-consent__run", "Run") as HTMLButtonElement;
  run.type = "button";
  run.addEventListener("click", () => {
    teardown();
    handlers.onRun();
  });
  actions.append(cancel, run);

  root.append(header, requests, notRequests, actions);
  container.append(root);
  return teardown;
}

/** Live handles to update the reader chrome. */
export interface ChromeHandle {
  root: El;
  setTitle(title: string): void;
  setDirty(dirty: boolean): void;
  setBadge(text: string | null): void;
  /** Marks the app unresponsive (watchdog tripped), surfacing Stop prominently. */
  setUnresponsive(unresponsive: boolean): void;
}

/**
 * Renders the reader window chrome: title bar with a dirty dot, a storage
 * badge, and a "Stop this application" button. The Stop button is wired to a
 * host-side teardown, so it works even when the application is hung.
 */
export function renderChrome(
  container: El,
  initial: { title: string; storageMode: string },
  onStop: () => void,
): ChromeHandle {
  const doc = container.ownerDocument;
  const bar = el(doc, "div", "poid-chrome");

  const dirtyDot = el(doc, "span", "poid-chrome__dirty");
  dirtyDot.style.visibility = "hidden";
  dirtyDot.setAttribute("aria-label", "unsaved changes");
  const titleEl = el(doc, "span", "poid-chrome__title", initial.title);
  const badge = el(doc, "span", "poid-chrome__badge", initial.storageMode);

  const stop = el(doc, "button", "poid-chrome__stop", "Stop this application") as HTMLButtonElement;
  stop.type = "button";
  stop.addEventListener("click", () => onStop());

  bar.append(dirtyDot, titleEl, badge, stop);
  container.append(bar);

  return {
    root: bar,
    setTitle: (title) => {
      titleEl.textContent = title;
    },
    setDirty: (dirty) => {
      dirtyDot.style.visibility = dirty ? "visible" : "hidden";
    },
    setBadge: (text) => {
      badge.textContent = text ?? initial.storageMode;
    },
    setUnresponsive: (unresponsive) => {
      bar.classList.toggle("poid-chrome--unresponsive", unresponsive);
    },
  };
}
