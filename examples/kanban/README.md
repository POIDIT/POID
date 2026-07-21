# Kanban

A kanban board whose entire data layer is `poid.db.docs` — the document tier
of the POID Data Engine (M10). It demonstrates the DoD: an app that stores its
cards in a document store, queries them, and survives a restart.

## What it shows

- **`poid.db.docs`** for everything: `collection("cards")` with `insert`,
  `find` (sorted), `update` (move a card between columns) and `delete`. No
  key-value, no hand-written SQL — the Mongo-like API a ported app targets.
- **Storage-mode independence.** The same file runs unchanged with
  `storage.mode` set to `embedded`, `vault`, or `connection`. The application
  never learns which backend answered; the reader decides.
- **Sandbox-correct UX.** Cards are added through an inline form, not
  `prompt()` — native modal dialogs are (correctly) disabled inside the
  sandboxed iframe.

## Build and run it

```sh
poid pack examples/kanban -o kanban.poid
```

Then open `kanban.poid` in POID Studio, or drop it onto the Web Reader
(https://poiddev.poidit.workers.dev). Add a few cards, close it, open it
again — the cards are still there.

## Manifest highlights

```jsonc
{
  "runtime":  { "profile": "web+sql" },       // the SQL-backed Data Engine
  "storage":  { "mode": "embedded",
                "schema_version": 1 }          // §12: migrations key off this
}
```

`profile: "web+sql"` declares that the app needs the SQL tier. Every
conformant reader provides it natively, so the file runs everywhere with no
download.

## Updating it (SPEC §12)

Ship a `v2` with a bumped `storage.schema_version` and a
`migrations/0002-*.sql` script, then:

```sh
poid update kanban.poid --from kanban-v2.poid
```

The program is swapped, the user's cards are kept, and the migration runs the
next time the file is opened.
