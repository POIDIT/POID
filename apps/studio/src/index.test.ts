import { describe, expect, it } from "vitest";
import { routeDocument } from "./desktop-flow.js";
import { type DocumentDto, decodeFiles } from "./document-dto.js";

function b64(text: string): string {
  return btoa(text);
}

const APP_MANIFEST = JSON.stringify({
  poid: "1.0",
  type: "app",
  app: { id: "com.example.kanban", name: "Kanban", version: "1.2.3" },
  instance: { id: null },
  runtime: { profile: "web" },
  entry: "app/index.html",
  storage: { mode: "embedded" },
  permissions: {},
});

function loaded(overrides: Partial<Extract<DocumentDto, { kind: "loaded" }>> = {}): DocumentDto {
  return {
    kind: "loaded",
    fileName: "kanban.poid",
    path: "C:\\docs\\kanban.poid",
    manifestJson: APP_MANIFEST,
    signature: "none",
    files: [{ path: "app/index.html", dataB64: b64("<!doctype html>") }],
    ...overrides,
  };
}

describe("decodeFiles", () => {
  it("round-trips paths and bytes", () => {
    const files = decodeFiles([
      { path: "app/index.html", dataB64: b64("hello") },
      { path: "data/store.json", dataB64: b64("{}") },
    ]);
    expect([...files.keys()]).toEqual(["app/index.html", "data/store.json"]);
    expect(new TextDecoder().decode(files.get("app/index.html"))).toBe("hello");
  });

  it("decodes binary content, not just text", () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 255]);
    const encoded = btoa(String.fromCharCode(...bytes));
    const files = decodeFiles([{ path: "app/x.bin", dataB64: encoded }]);
    expect([...(files.get("app/x.bin") ?? [])]).toEqual([0, 1, 2, 250, 255]);
  });
});

describe("routeDocument", () => {
  it("routes a rejection with the human explanation", () => {
    const outcome = routeDocument({
      kind: "rejected",
      registry: "POID-020",
      code: "native-code",
      message: "app/tool.exe",
      fileName: "bad.poid",
    });
    expect(outcome.kind).toBe("rejected");
    if (outcome.kind === "rejected") {
      expect(outcome.explanation).toContain("native");
      expect(outcome.registry).toBe("POID-020");
    }
  });

  it("routes a plain web app as runnable and assigns instance.id", () => {
    const outcome = routeDocument(loaded());
    expect(outcome.kind).toBe("runnable");
    if (outcome.kind === "runnable") {
      expect(outcome.facts.instanceId).toMatch(/^[0-9a-f-]{36}$/);
      expect(outcome.files.has("app/index.html")).toBe(true);
    }
  });

  it("keeps an existing instance.id", () => {
    const manifest = JSON.parse(APP_MANIFEST);
    manifest.instance = { id: "11111111-2222-4333-8444-555555555555" };
    const outcome = routeDocument(loaded({ manifestJson: JSON.stringify(manifest) }));
    if (outcome.kind !== "runnable") throw new Error(`expected runnable, got ${outcome.kind}`);
    expect(outcome.facts.instanceId).toBe("11111111-2222-4333-8444-555555555555");
  });

  it("routes a non-web profile to an honest engine notice", () => {
    const manifest = JSON.parse(APP_MANIFEST);
    manifest.runtime = { profile: "web+python", engines: { pyodide: ">=0.26" } };
    const outcome = routeDocument(loaded({ manifestJson: JSON.stringify(manifest) }));
    expect(outcome.kind).toBe("engine-missing");
    if (outcome.kind === "engine-missing") {
      expect(outcome.missingEngines).toEqual(["pyodide"]);
    }
  });

  it("routes data containers to a notice, never to execution", () => {
    const manifest = {
      poid: "1.0",
      type: "data",
      data_ref: { app_id: "com.example.kanban", app_version: "1.2.3", schema: "v1" },
    };
    const outcome = routeDocument(loaded({ manifestJson: JSON.stringify(manifest) }));
    expect(outcome.kind).toBe("data-container");
  });
});
