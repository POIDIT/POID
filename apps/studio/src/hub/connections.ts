/**
 * The Connections panel (M11.3, moved into the shell in M12.1).
 *
 * The one rule this file exists to keep: **a credential travels one way.** The
 * secret field is write-only — there is no IPC command that returns a stored
 * credential, so editing a connection replaces the secret rather than
 * revealing it. That costs the user one retype and removes a whole class of
 * disclosure permanently.
 *
 * The markup moved from `static/index.html` into this file when the hub became
 * a shell: a panel that cannot be added or removed without editing a shared
 * HTML page is not really a panel.
 */

import { invoke } from "@tauri-apps/api/core";
import { button, el } from "./dom.js";
import type { Panel } from "./panels.js";

/** One connection as the backend describes it. Note the absent secret. */
interface ConnectionDto {
  id: string;
  kind: string;
  label: string;
  target: string;
  hasSecret: boolean;
}

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

/** Builds one labelled field. */
function field(labelText: string, control: HTMLElement, id?: string): HTMLLabelElement {
  const wrapper = el("label", "conn-form__field");
  if (id) wrapper.id = id;
  wrapper.append(el("span", undefined, labelText), control);
  return wrapper;
}

export const connectionsPanel: Panel = {
  id: "connections",
  label: "Connections",
  title: "Connections",
  blurb:
    "Databases and services your POIDs may ask to use. A POID never learns the password — it " +
    "asks for data, and Studio makes the request. Passwords are kept in this computer's own " +
    "credential store, never inside a POID and never in a file.",

  mount(container) {
    // ---------------------------------------------------------------- markup

    const add = button("conn__add", "Add a connection…");
    add.id = "conn-add";

    const list = el("div", "conn__list");
    list.id = "conn-list";
    list.setAttribute("aria-live", "polite");

    const kindSelect = el("select");
    kindSelect.id = "conn-kind";
    kindSelect.name = "kind";
    for (const [value, text] of [
      ["sql", "A database (PostgreSQL, Supabase, Neon…)"],
      ["net", "An API key for a website"],
    ] as const) {
      const option = el("option", undefined, text);
      option.value = value;
      kindSelect.append(option);
    }

    const labelInput = el("input");
    labelInput.id = "conn-label";
    labelInput.name = "label";
    labelInput.type = "text";
    labelInput.placeholder = "my-supabase";
    labelInput.required = true;

    const originsInput = el("textarea");
    originsInput.id = "conn-origins";
    originsInput.name = "origins";
    originsInput.rows = 2;
    originsInput.placeholder = "https://api.example.com";

    const secretInput = el("input");
    secretInput.id = "conn-secret";
    secretInput.name = "secret";
    secretInput.type = "password";
    secretInput.autocomplete = "off";
    secretInput.spellcheck = false;

    const secretLabel = el("span", undefined, SQL_WORDS.secret);
    secretLabel.id = "conn-secret-label";
    const secretHint = el("small", "conn-form__hint", SQL_WORDS.hint);
    secretHint.id = "conn-secret-hint";
    const secretField = el("label", "conn-form__field");
    secretField.append(secretLabel, secretInput, secretHint);

    const originsField = field(
      "Which addresses may use it? One per line.",
      originsInput,
      "conn-origins-field",
    );

    const formTitle = el("h2", "conn-form__title", "Add a connection");
    formTitle.id = "conn-form-title";

    const errorLine = el("p", "conn-form__error");
    errorLine.id = "conn-error";
    errorLine.hidden = true;

    const cancel = button(undefined, "Cancel");
    cancel.id = "conn-cancel";
    const saveButton = button("poid-drop__pick", "Save");
    saveButton.id = "conn-save";
    const actions = el("div", "conn-form__actions");
    actions.append(cancel, saveButton);

    const form = el("form", "conn-form");
    form.id = "conn-form";
    form.method = "dialog";
    form.append(
      formTitle,
      field("What is it?", kindSelect),
      field("Name it, so you recognise it later", labelInput),
      originsField,
      secretField,
      errorLine,
      actions,
    );

    const dialog = el("dialog", "conn-dialog");
    dialog.id = "conn-dialog";
    dialog.append(form);

    const head = el("div", "conn__head");
    head.append(add);

    container.append(head, list, dialog);

    // ------------------------------------------------------------- behaviour

    /** The connection being edited, or null when adding a new one. */
    let editing: ConnectionDto | null = null;

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

        // A keychain item can be deleted from outside Studio. Saying so here
        // beats failing at the moment the user tries to open a document with it.
        if (!connection.hasSecret) {
          main.append(
            el(
              "span",
              "conn__missing",
              "Password missing — it is no longer in this computer's credential store. Edit to set it again.",
            ),
          );
        }

        const rowActions = el("div", "conn__actions");
        const edit = button(undefined, "Edit");
        edit.addEventListener("click", () => openDialog(connection));

        const remove = button("conn__delete", "Delete");
        remove.addEventListener("click", () => void removeConnection(connection));

        rowActions.append(edit, remove);
        row.append(main, rowActions);
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
      originsInput.value =
        connection?.kind === "net" ? connection.target.split(", ").join("\n") : "";

      // Always blank, for both add and edit: Studio does not have the old value
      // to put here, and showing a placeholder that looked like one would teach
      // the user that it does.
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
          fail(
            kind === "sql" ? "Paste the connection string for the database." : "Paste the API key.",
          );
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
        // Cleared the moment it is stored: no reason for a credential to sit in
        // an input for the rest of the session.
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

    add.addEventListener("click", () => openDialog(null));
    cancel.addEventListener("click", () => dialog.close());
    saveButton.addEventListener("click", () => void save());
    kindSelect.addEventListener("change", () => applyKind(kindSelect.value));
    dialog.addEventListener("close", () => {
      secretInput.value = "";
    });

    applyKind("sql");
    void refresh();
    return undefined;
  },
};
