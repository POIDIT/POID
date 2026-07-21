# POID Runtime API (`window.poid`)

**Version:** 1.0.0-draft.1

This is the contract between a POID application and the reader. It is the **real** standard — the container layout is just packaging. Any conformant reader on any platform MUST expose exactly this surface, with identical semantics.

---

## 0. Fundamentals

- The API is injected as a global `poid` object inside the sandboxed iframe.
- **Every method is async** and returns a `Promise`.
- Every call is a `postMessage` to the host, brokered by Core.
- **The application never receives credentials.** Ever. See §7 of `SPEC.md`.
- A call to an undeclared or ungranted capability rejects with `PoidPermissionError`. It does not silently no-op.

```ts
declare const poid: PoidAPI;
```

---

## 1. `poid.app` — identity and lifecycle

```ts
poid.app.info(): Promise<{
  id: string;
  name: string;
  version: string;
  instanceId: string;
  readerVersion: string;
  platform: "windows" | "macos" | "linux" | "ios" | "android" | "web";
  storageMode: "embedded" | "vault" | "connection";
  slot: string | null;
}>

poid.app.onSuspend(cb: () => void | Promise<void>): void
// Fired before the window closes or the app is backgrounded.
// The app MUST flush unsaved state here. The reader waits (with a timeout).

poid.app.setTitle(title: string): Promise<void>
poid.app.setDirty(dirty: boolean): Promise<void>
// Tells the reader whether there are unsaved changes (shows a dot in the title bar,
// prompts on close). Standard document behaviour.
```

---

## 2. `poid.db` — the Data Engine

### 2.1 Key-value

```ts
poid.db.kv.get<T>(key: string): Promise<T | null>
poid.db.kv.set<T>(key: string, value: T): Promise<void>
poid.db.kv.delete(key: string): Promise<void>
poid.db.kv.list(prefix?: string): Promise<string[]>
poid.db.kv.clear(): Promise<void>
```

Keys: max 256 chars, no whitespace, no `/ \ ' "`.
Values: JSON-serialisable, max 5 MB per key.

### 2.2 SQL

```ts
poid.db.sql.exec(sql: string, params?: unknown[]): Promise<{
  rows: Record<string, unknown>[];
  rowsAffected: number;
}>

poid.db.sql.transaction(fn: (tx: SqlTx) => Promise<void>): Promise<void>
```

Backed by wa-sqlite (WASM) locally, or by a Connection when `storage.mode = "connection"`.
The application **does not** know or care which. **The broker executes the query.**

Requires `runtime.profile` to include `sql`, or `storage.mode = "connection"` with `kind: "sql"`.

### 2.3 Documents

```ts
poid.db.docs.collection(name: string): {
  insert<T>(doc: T): Promise<{ _id: string }>;
  find<T>(query?: Query, opts?: { limit?: number; skip?: number; sort?: Sort }): Promise<T[]>;
  findOne<T>(query: Query): Promise<T | null>;
  update(query: Query, patch: object): Promise<{ matched: number }>;
  delete(query: Query): Promise<{ deleted: number }>;
  count(query?: Query): Promise<number>;
}
```

`Query` supports a Mongo-like subset: `$eq $ne $gt $gte $lt $lte $in $nin $and $or $exists $regex`.
Implemented over SQLite with a JSON index. **This is the migration target for apps that used MongoDB.**

Semantics (normative):

- **`_id`** is a string. If `insert` receives a document without one, the reader assigns a UUIDv4 and returns it. It is unique within the collection; a duplicate `_id` is rejected. `_id` is immutable — it cannot be changed by `update`.
- **`update(query, patch)` is a shallow merge** (Mongo `$set`): each top-level key of `patch` replaces that field, other fields are untouched. A patch value that is an object replaces the whole field (not a deep merge). Update operators (`$set`, `$inc`, …) are **not** supported — pass plain fields. Dotted keys (`"meta.color"`) address a nested field.
- **Missing vs null**: equality treats a missing field and a JSON `null` alike (`{f: null}` matches both), and `$ne` / `$nin` match documents where the field is absent — as in MongoDB.
- **Types are not coerced**: `{done: true}` does not match a stored integer `1`, and `{n: 1}` does not match a stored `true`.
- **Values** are JSON scalars. `$regex` takes a JavaScript regular-expression source string.

### 2.4 Slots

```ts
poid.db.slots.list(): Promise<string[]>
poid.db.slots.current(): Promise<string>
```

Read-only for the application. **Switching slots is the reader's job, not the app's** — a malicious app must not be able to read another slot.

---

## 3. `poid.files` — user-initiated file access

```ts
poid.files.open(opts?: {
  accept?: string[];     // [".csv", ".json"]
  multiple?: boolean;
}): Promise<File[]>

poid.files.save(blob: Blob, suggestedName: string): Promise<void>
```

Requires `permissions.filesystem = "user-initiated"`.
**Both methods MUST open a native OS dialog.** There is no path-based filesystem access, and there never will be. The app can only ever touch what the user explicitly picked.

---

## 4. `poid.net` — brokered network access

```ts
poid.net.fetch(url: string, init?: RequestInit): Promise<Response>
```

- The origin **MUST** appear in `permissions.network` **and** be user-approved.
- The broker performs the request. It strips `Authorization` headers set by the app and injects credentials only if the origin maps to a Connection.
- Anything else is blocked by CSP before it reaches the broker at all.

An app with `"network": []` has no `poid.net`. The property is undefined.

---

## 5. `poid.ai` — model access without keys

```ts
poid.ai.complete(opts: {
  messages: { role: "user" | "assistant" | "system"; content: string }[];
  model?: string;         // a logical name; the reader maps it to a provider
  maxTokens?: number;
  stream?: boolean;
}): Promise<AsyncIterable<string> | string>

poid.ai.models(): Promise<string[]>
```

The user configures an AI Connection once, in Studio (their own key, or POID Cloud AI Gateway).
**The application never sees the key.** It asks for a completion; the broker calls the provider.

This is what makes "an AI app you can email someone" safe: the recipient plugs in *their* key, and the file has no way to exfiltrate it.

---

## 6. `poid.mcp` — external tool access

```ts
poid.mcp.servers(): Promise<{ id: string; name: string; tools: string[] }[]>
poid.mcp.call(serverId: string, tool: string, args: object): Promise<unknown>
```

- Only servers listed in `permissions.mcp` and approved by the user.
- **HTTP/SSE transport only.** `stdio` (local process spawning) is **not** available to applications — ever.
- OAuth happens in Studio (Connections). The app receives results, never tokens.
- The user may revoke any server or tool at any time.

---

## 7. `poid.ui` — reader-provided chrome

```ts
poid.ui.notify(msg: string, level?: "info" | "warn" | "error"): Promise<void>
poid.ui.confirm(msg: string): Promise<boolean>
poid.ui.setBadge(text: string | null): Promise<void>
poid.ui.print(): Promise<void>              // requires permissions.print
poid.ui.clipboard.write(text: string): Promise<void>  // requires permissions.clipboard
poid.ui.clipboard.read(): Promise<string>             // requires permissions.clipboard + user gesture
```

---

## 8. `poid.export` — data portability

```ts
poid.export.data(payload: object, opts?: { schema?: string }): Promise<void>
// Produces a `type: data` POID and hands it to the reader's save dialog.
// This is the survey-response flow (SPEC §11).

poid.export.pdf(opts?: { title?: string }): Promise<void>
// Renders the current view to PDF. Archival, printing.
```

---

## 9. Errors

```ts
class PoidError extends Error { code: string }
```

| Code | Meaning |
|---|---|
| `PERMISSION_DENIED` | Capability not declared, or not granted by the user |
| `QUOTA_EXCEEDED` | Storage quota exhausted |
| `NOT_AVAILABLE` | Capability not supported by this reader / platform |
| `CONNECTION_REQUIRED` | `storage.mode = "connection"` but none configured |
| `INVALID_ARGUMENT` | Bad input |
| `INTERNAL` | Reader-side failure |

Error messages **MUST NOT** contain credentials, absolute host paths, or vault internals.

---

## 10. Reference: a complete minimal app

```html
<!doctype html>
<html>
<body>
  <textarea id="note" style="width:100%;height:90vh"></textarea>
  <script type="module">
    const el = document.getElementById("note");
    el.value = (await poid.db.kv.get("note")) ?? "";

    let t;
    el.addEventListener("input", () => {
      poid.app.setDirty(true);
      clearTimeout(t);
      t = setTimeout(async () => {
        await poid.db.kv.set("note", el.value);
        poid.app.setDirty(false);
      }, 400);
    });

    poid.app.onSuspend(() => poid.db.kv.set("note", el.value));
  </script>
</body>
</html>
```

Manifest: `type: app`, `profile: web`, `storage.mode: embedded`, `permissions: { network: [], filesystem: "none" }`.

That is a complete, shareable, offline, secure POID. Email it; the recipient sees the notes.
