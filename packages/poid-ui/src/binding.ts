/**
 * The connection-choice prompt (SPEC §7.2.3).
 *
 * Shown after consent and before the application exists — the same ordering,
 * and for the same reason, as the consent screen itself: nothing the container
 * ships can style, cover, pre-answer or race a dialog that is drawn in the
 * host document while the iframe has not been created.
 *
 * The rule this file encodes, beyond drawing boxes: **"keep it here" is always
 * an option.** A reader that offered only "connect or close" would make the
 * manifest's declaration into a demand, and SPEC §7.2.3 clause 2 exists
 * precisely because that is the shape this feature would otherwise drift into.
 */

import { el } from "./dom.js";

/** One connection the user could pick. Carries no credential — the reader
 * never has one to carry (SPEC §7.1). */
export interface BindingCandidate {
  id: string;
  /** The user's own name for it. */
  label: string;
  /** A non-secret one-line description of where it points. */
  target: string;
  /** Whether its credential is still in the OS credential store. */
  hasSecret: boolean;
}

/** What the prompt needs to know. */
export interface BindingModel {
  /** The application's name, for the sentence. */
  appName: string;
  /** What it declared it needs: `storage.requires.kind` (SPEC §3.1). */
  need: "kv" | "sql" | "docs" | "files";
  /** Connections that could serve it, already filtered and ordered. */
  candidates: BindingCandidate[];
  /**
   * Whether this reader can hold credentials at all.
   *
   * `false` in the Web Reader, which has no OS credential store — and which
   * therefore must say so rather than offering a weaker place to put a
   * password (SPEC §7.2.2).
   */
  canConnect: boolean;
}

/** The user's answer. */
export type BindingChoice = { kind: "connection"; id: string } | { kind: "local" };

/** Callbacks the prompt needs from the reader. */
export interface BindingHandlers {
  onChoose(choice: BindingChoice): void;
  /** Opens the place where connections are configured (the Studio hub). */
  onConfigure?(): void;
  /** Re-reads the connection list, after the user has been to configure one. */
  onRefresh?(): void;
}

const NEED_WORDS: Record<BindingModel["need"], string> = {
  sql: "an external database",
  docs: "an external document store",
  kv: "an external key-value store",
  files: "external file storage",
};

/**
 * Renders the prompt and returns a teardown function.
 *
 * Nothing from the container can call the handlers: this runs in the host
 * document, and the application has no reference to these nodes.
 */
export function renderBindingChoice(
  container: HTMLElement,
  model: BindingModel,
  handlers: BindingHandlers,
): () => void {
  const doc = container.ownerDocument;
  const root = el(doc, "div", "poid-binding");
  root.setAttribute("role", "dialog");
  root.setAttribute("aria-modal", "true");

  const teardown = () => root.remove();
  const choose = (choice: BindingChoice) => {
    teardown();
    handlers.onChoose(choice);
  };

  root.append(
    el(doc, "h1", "poid-binding__title", "Where should this store its data?"),
    el(
      doc,
      "p",
      "poid-binding__lead",
      `“${model.appName}” is built to keep its data in ${NEED_WORDS[model.need]} rather than inside the file.`,
    ),
  );

  const actions = el(doc, "div", "poid-binding__actions");

  if (!model.canConnect) {
    // The honest version for a reader with no credential store. Saying "not
    // here" is the conformant answer; quietly keeping the password somewhere
    // weaker is not (SPEC §7.2.2).
    root.append(
      el(
        doc,
        "p",
        "poid-binding__note",
        "This browser version cannot hold database passwords safely, so it will not ask you for one. The application will work, keeping its data in this browser. Install POID Studio to connect it to a real database.",
      ),
    );
    const keep = el(doc, "button", "poid-binding__primary", "Keep the data in this browser");
    (keep as HTMLButtonElement).type = "button";
    keep.addEventListener("click", () => choose({ kind: "local" }));
    actions.append(keep);
    root.append(actions);
    container.append(root);
    return teardown;
  }

  const usable = model.candidates.filter((c) => c.hasSecret);
  const unusable = model.candidates.filter((c) => !c.hasSecret);

  if (usable.length > 0) {
    const list = el(doc, "ul", "poid-binding__list");
    for (const candidate of usable) {
      const item = el(doc, "li");
      const pick = el(doc, "button", "poid-binding__candidate") as HTMLButtonElement;
      pick.type = "button";
      pick.append(
        el(doc, "span", "poid-binding__candidate-label", candidate.label),
        el(doc, "span", "poid-binding__candidate-target", candidate.target),
      );
      pick.addEventListener("click", () => choose({ kind: "connection", id: candidate.id }));
      item.append(pick);
      list.append(item);
    }
    root.append(el(doc, "h2", "poid-binding__subtitle", "Use one you have set up:"), list);
  } else {
    root.append(
      el(
        doc,
        "p",
        "poid-binding__note",
        "You have not set up a database that can hold this. You can add one now, or let the application keep its data on this computer — it will work either way.",
      ),
    );
  }

  // A connection whose keychain entry has gone is listed but not offered:
  // silently omitting it would look like the user never configured it.
  for (const candidate of unusable) {
    root.append(
      el(
        doc,
        "p",
        "poid-binding__stale",
        `“${candidate.label}” is set up, but its password is no longer stored on this computer. Fix it in Connections to use it here.`,
      ),
    );
  }

  if (handlers.onConfigure) {
    const configure = el(doc, "button", undefined, "Set one up…") as HTMLButtonElement;
    configure.type = "button";
    configure.addEventListener("click", () => handlers.onConfigure?.());
    actions.append(configure);
  }
  if (handlers.onRefresh) {
    const refresh = el(doc, "button", undefined, "Check again") as HTMLButtonElement;
    refresh.type = "button";
    refresh.addEventListener("click", () => {
      teardown();
      handlers.onRefresh?.();
    });
    actions.append(refresh);
  }

  // Always present, always last, never styled as the reluctant option.
  const keep = el(
    doc,
    "button",
    usable.length > 0 ? undefined : "poid-binding__primary",
    "Keep the data on this computer",
  ) as HTMLButtonElement;
  keep.type = "button";
  keep.addEventListener("click", () => choose({ kind: "local" }));
  actions.append(keep);

  root.append(actions);
  container.append(root);
  return teardown;
}
