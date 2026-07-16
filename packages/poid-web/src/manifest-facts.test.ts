import { describe, expect, it } from "vitest";
import { consentManifestFrom, extractFacts, hostFacts, runGrant } from "./manifest-facts.js";

const APP_MANIFEST = JSON.stringify({
  poid: "1.0",
  type: "app",
  app: { id: "com.example.kanban", name: "Kanban", version: "1.2.3", author: "Jane" },
  instance: { id: null },
  runtime: { profile: "web+python", engines: { pyodide: ">=0.26 <0.28" } },
  entry: "app/index.html",
  storage: { mode: "embedded", slots: true, protected: true },
  permissions: {
    network: ["https://api.example.com"],
    filesystem: "user-initiated",
    clipboard: true,
    mcp: [],
  },
});

describe("extractFacts", () => {
  it("reads the fields the reader needs", () => {
    const facts = extractFacts(APP_MANIFEST);
    expect(facts.type).toBe("app");
    expect(facts.name).toBe("Kanban");
    expect(facts.version).toBe("1.2.3");
    expect(facts.author).toBe("Jane");
    expect(facts.entry).toBe("app/index.html");
    expect(facts.instanceId).toBeNull();
    expect(facts.profile).toBe("web+python");
    expect(facts.engines).toEqual({ pyodide: ">=0.26 <0.28" });
    expect(facts.storageMode).toBe("embedded");
    expect(facts.slots).toBe(true);
    expect(facts.protectedData).toBe(true);
    expect(facts.permissions.network).toEqual(["https://api.example.com"]);
    expect(facts.permissions.filesystem).toBe("user-initiated");
    expect(facts.permissions.clipboard).toBe(true);
    expect(facts.permissions.print).toBe(false);
  });

  it("applies spec defaults when optional blocks are absent", () => {
    const facts = extractFacts(
      JSON.stringify({
        poid: "1.0",
        type: "app",
        app: { id: "com.example.min", name: "Min", version: "1.0.0" },
        entry: "app/index.html",
      }),
    );
    expect(facts.storageMode).toBe("embedded");
    expect(facts.slots).toBe(false);
    expect(facts.protectedData).toBe(false);
    expect(facts.profile).toBe("web");
    expect(facts.permissions).toEqual({
      network: [],
      filesystem: "none",
      clipboard: false,
      print: false,
      notifications: false,
      mcp: [],
    });
  });

  it("names a data container after the application it belongs to", () => {
    const facts = extractFacts(
      JSON.stringify({
        poid: "1.0",
        type: "data",
        data_ref: { app_id: "com.example.survey", app_version: "1.0.0", schema: "responses/v1" },
      }),
    );
    expect(facts.type).toBe("data");
    expect(facts.name).toBe("com.example.survey");
    expect(facts.dataRef).toEqual({
      appId: "com.example.survey",
      appVersion: "1.0.0",
      schema: "responses/v1",
    });
  });
});

describe("consent and grant shaping", () => {
  it("builds the consent manifest with the signature state", () => {
    const consent = consentManifestFrom(extractFacts(APP_MANIFEST), "valid");
    expect(consent.name).toBe("Kanban");
    expect(consent.signature).toBe("valid");
    expect(consent.permissions.network).toEqual(["https://api.example.com"]);
  });

  it("Run approves exactly what was requested, and never AI", () => {
    const facts = extractFacts(APP_MANIFEST);
    const grant = runGrant(facts);
    expect(grant.network).toEqual(["https://api.example.com"]);
    expect(grant.filesystem).toBe(true);
    expect(grant.clipboard).toBe(true);
    expect(grant.print).toBe(false);
    expect(grant.ai).toBe(false);
  });

  it("shapes host facts for capabilitiesFromGrant", () => {
    const facts = hostFacts(extractFacts(APP_MANIFEST));
    expect(facts.storageMode).toBe("embedded");
    expect(facts.storageSlots).toBe(true);
    expect(facts.profile).toBe("web+python");
  });
});
