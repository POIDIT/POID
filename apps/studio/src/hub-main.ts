/**
 * The Studio hub window: opening a POID, and managing Connections (M11.3).
 *
 * The one rule this file exists to keep: **a credential travels one way.** The
 * secret field is write-only — there is no IPC command that returns a stored
 * credential, so editing a connection replaces the secret rather than
 * revealing it. That costs the user one retype and removes a whole class of
 * disclosure permanently.
 *
 * Opening a document still creates a separate Reader window, never a panel
 * inside the hub (ARCHITECTURE §2).
 */

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

/** One connection as the backend describes it. Note the absent secret. */
interface ConnectionDto {
  id: string;
  kind: string;
  label: string;
  target: string;
  hasSecret: boolean;
}

function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`studio hub is missing #${id}`);
  return node as T;
}

function el(tag: string, className?: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// ------------------------------------------------------------- open a POID

byId("open-poid").addEventListener("click", () => {
  void (async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "POID Document", extensions: ["poid"] }],
    });
    if (typeof picked === "string") {
      await invoke("open_document", { path: picked });
    }
  })();
});

// ------------------------------------------------------------- connections

const list = byId("conn-list");
const dialog = byId<HTMLDialogElement>("conn-dialog");
const formTitle = byId("conn-form-title");
const kindSelect = byId<HTMLSelectElement>("conn-kind");
const labelInput = byId<HTMLInputElement>("conn-label");
const secretInput = byId<HTMLInputElement>("conn-secret");
const secretLabel = byId("conn-secret-label");
const secretHint = byId("conn-secret-hint");
const originsField = byId("conn-origins-field");
const originsInput = byId<HTMLTextAreaElement>("conn-origins");
const errorLine = byId("conn-error");

/** The connection being edited, or null when adding a new one. */
let editing: ConnectionDto | null = null;

interface KindWords {
  noun: string;
  secret: string;
  hint: string;
  placeholder: string;
}

/** Human wording per kind. Errors and labels are written for a person, not a
 * developer (CONVENTIONS, UX rule 4). */
const SQL_WORDS: KindWords = {
  noun: "Database",
  secret: "Connection string",
  hint: "This goes straight into your computer's credential store. Studio cannot show it back to you afterwards — that is deliberate.",
  placeholder: "postgres://user:password@host:5432/database",
};

const NET_WORDS: KindWords = {
  noun: "API key",
  secret: "API key or token",
  hint: "Only the addresses above will ever receive this key, and a POID never sees it.",
  placeholder: "sk-…",
};

/** Resolved by comparison rather than by index, so an unknown kind falls back
 * to sensible wording instead of to `undefined`. */
function wordsFor(kind: string): KindWords {
  return kind === "net" ? NET_WORDS : SQL_WORDS;
}

function renderList(connections: ConnectionDto[]): void {
  list.replaceChildren();

  if (connections.length === 0) {
    list.append(
      el(
        "p",
        "conn__empty",
        "Nothing set up yet. A POID that wants a database will ask, and you can also add one here in advance — either way, you choose which one it gets, and it can always be told to keep its data on this computer instead.",
      ),
    );
    return;
  }

  for (const connection of connections) {
    const row = el("div", "conn__row");
    const noun = wordsFor(connection.kind).noun;

    const main = el("div", "conn__main");
    main.append(
      el("span", "conn__label", connection.label),
      el("span", "conn__kind", noun),
      el("span", "conn__target", connection.target),
    );

    // A keychain item can be deleted from outside Studio. Saying so here beats
    // failing at the moment the user tries to open a document with it.
    if (!connection.hasSecret) {
      main.append(
        el(
          "span",
          "conn__missing",
          "Password missing — it is no longer in this computer's credential store. Edit to set it again.",
        ),
      );
    }

    const actions = el("div", "conn__actions");
    const edit = el("button", undefined, "Edit") as HTMLButtonElement;
    edit.type = "button";
    edit.addEventListener("click", () => openDialog(connection));

    const remove = el("button", "conn__delete", "Delete") as HTMLButtonElement;
    remove.type = "button";
    remove.addEventListener("click", () => void removeConnection(connection));

    actions.append(edit, remove);
    row.append(main, actions);
    list.append(row);
  }
}

async function refresh(): Promise<void> {
  try {
    renderList(await invoke<ConnectionDto[]>("connections_list"));
  } catch (e) {
    list.replaceChildren(
      el("p", "conn__empty", `The list of connections could not be read. ${String(e)}`),
    );
  }
}

function applyKind(kind: string): void {
  const words = wordsFor(kind);
  secretLabel.textContent = words.secret;
  secretHint.textContent = words.hint;
  secretInput.placeholder = words.placeholder;
  originsField.hidden = kind !== "net";
}

function openDialog(connection: ConnectionDto | null): void {
  editing = connection;
  errorLine.hidden = true;

  formTitle.textContent = connection ? `Edit “${connection.label}”` : "Add a connection";
  kindSelect.value = connection?.kind ?? "sql";
  // Changing what a connection *is* means making a different one; editing
  // keeps the kind so the stored credential still means what it meant.
  kindSelect.disabled = connection !== null;
  labelInput.value = connection?.label ?? "";
  originsInput.value = connection?.kind === "net" ? connection.target.split(", ").join("\n") : "";

  // Always blank, for both add and edit: Studio does not have the old value to
  // put here, and showing a placeholder that looked like one would teach the
  // user that it does.
  secretInput.value = "";
  applyKind(kindSelect.value);
  if (connection) {
    secretHint.textContent = connection.hasSecret
      ? "Leave this blank to keep the password you already stored, or type a new one to replace it."
      : "The stored password is gone. Type it again to make this connection usable.";
  }

  dialog.showModal();
  labelInput.focus();
}

function fail(message: string): void {
  errorLine.textContent = message;
  errorLine.hidden = false;
}

async function save(): Promise<void> {
  const kind = kindSelect.value;
  const label = labelInput.value.trim();
  const secret = secretInput.value;
  const origins = originsInput.value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (!label) {
    fail("Give the connection a name so you can recognise it later.");
    return;
  }

  // A blank secret while editing means "keep what is stored". The backend
  // never hands the old value back, so there is nothing to re-send — which
  // makes renaming the only change possible without a fresh credential.
  if (!secret) {
    if (!editing) {
      fail(kind === "sql" ? "Paste the connection string for the database." : "Paste the API key.");
      return;
    }
    if (!editing.hasSecret) {
      fail("This connection has no stored password, so one has to be typed here.");
      return;
    }
    try {
      await invoke("connection_rename", { id: editing.id, label });
      dialog.close();
      await refresh();
    } catch (e) {
      fail(String(e));
    }
    return;
  }

  try {
    await invoke<ConnectionDto>("connection_save", {
      id: editing?.id ?? null,
      kind,
      label,
      secret,
      origins,
    });
    // Cleared the moment it is stored: no reason for a credential to sit in an
    // input for the rest of the session.
    secretInput.value = "";
    dialog.close();
    await refresh();
  } catch (e) {
    fail(String(e));
  }
}

async function removeConnection(connection: ConnectionDto): Promise<void> {
  // Destructive and not undoable, so it is confirmed (CONVENTIONS, UX rule 3).
  const ok = window.confirm(
    `Delete “${connection.label}”?\n\nThe stored password is removed from this computer's credential store too. Any POID using this connection will ask you again next time it opens.`,
  );
  if (!ok) return;
  try {
    await invoke("connection_remove", { id: connection.id });
    await refresh();
  } catch (e) {
    fail(String(e));
  }
}

byId("conn-add").addEventListener("click", () => openDialog(null));
byId("conn-cancel").addEventListener("click", () => dialog.close());
byId("conn-save").addEventListener("click", () => void save());
kindSelect.addEventListener("change", () => applyKind(kindSelect.value));
dialog.addEventListener("close", () => {
  secretInput.value = "";
});

applyKind("sql");
void refresh();
