import { describe, expect, it } from "vitest";
import { type ConsentManifest, consentModel } from "./consent.js";

function manifest(overrides: Partial<ConsentManifest["permissions"]> = {}): ConsentManifest {
  return {
    name: "Kanban",
    version: "1.0.0",
    author: "Jane Doe",
    signature: "none",
    permissions: {
      network: [],
      filesystem: "none",
      clipboard: false,
      print: false,
      notifications: false,
      mcp: [],
      ...overrides,
    },
  };
}

describe("consent model", () => {
  it("always lists storage as requested and credentials as not-requested", () => {
    const m = consentModel(manifest());
    expect(m.requests).toContain("Store data in this file");
    expect(m.notRequests).toContain("Access to your credentials");
  });

  it("lists an unrequested capability under 'does not request'", () => {
    const m = consentModel(manifest());
    expect(m.notRequests).toContain("Internet access");
    expect(m.notRequests).toContain("Access to your files");
    expect(m.requests).not.toContain("Internet access");
  });

  it("lists a requested capability under 'requests', with detail", () => {
    const m = consentModel(
      manifest({ network: ["https://api.example.com"], filesystem: "user-initiated" }),
    );
    expect(m.requests).toContain("Internet access: https://api.example.com");
    expect(m.requests).toContain("Open and save files you choose");
    expect(m.notRequests).not.toContain("Internet access");
  });
});
