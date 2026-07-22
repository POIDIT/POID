/**
 * The Studio hub window: a shell that shows one panel at a time.
 *
 * The hub owns navigation and nothing else. Every tool — opening a document,
 * connections, and the converter, editor, library, storage and runtime panels
 * that follow in M12 — is a `Panel` that renders its own markup into an empty
 * container. Adding a tool means adding it to `PANELS`; it means no edit to
 * `index.html` and no edit to this file's logic.
 *
 * Opening a document still creates a separate Reader window, never a panel
 * inside the hub (ARCHITECTURE §2).
 */

import { connectionsPanel } from "./hub/connections.js";
import { el } from "./hub/dom.js";
import { homePanel } from "./hub/home.js";
import { hashForPanel, type Panel, panelIdFromHash, type Teardown } from "./hub/panels.js";

/** Every tool in the hub, in the order the navigation lists them. The first
 * is what a fresh window shows. */
const PANELS: readonly Panel[] = [homePanel, connectionsPanel];

function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`studio hub is missing #${id}`);
  return node as T;
}

const nav = byId("hub-nav");
const panelRoot = byId("hub-panel");

/** Stops whatever the panel on screen started. */
let teardown: Teardown | undefined;

const links = new Map<string, HTMLAnchorElement>();
for (const panel of PANELS) {
  // A real anchor, not a button: it gives keyboard navigation, the browser's
  // own history, and a hash the e2e tier can jump straight to.
  const link = el("a", "hub-nav__item", panel.label);
  link.href = hashForPanel(panel.id);
  links.set(panel.id, link);
  nav.append(link);
}

function show(id: string): void {
  teardown?.();
  teardown = undefined;

  const panel = PANELS.find((candidate) => candidate.id === id);
  if (!panel) return;

  for (const [panelId, link] of links) {
    if (panelId === panel.id) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }

  const title = el("h1", "hub__title", panel.title);
  title.id = "panel-title";
  const head = el("header", "hub-panel__head");
  head.append(title, el("p", "hub__hint", panel.blurb));

  const body = el("div", "hub-panel__body");
  panelRoot.replaceChildren(head, body);

  teardown = panel.mount(body);
}

const ids = PANELS.map((panel) => panel.id);
window.addEventListener("hashchange", () => show(panelIdFromHash(window.location.hash, ids)));
show(panelIdFromHash(window.location.hash, ids));
