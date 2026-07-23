import { describe, expect, it } from "vitest";
import { hashForPanel, panelIdFromHash } from "./panels.js";

const KNOWN = ["home", "connections"] as const;

describe("panelIdFromHash", () => {
  it("selects the panel named by the hash", () => {
    expect(panelIdFromHash("#connections", KNOWN)).toBe("connections");
  });

  it("accepts a hash without its #", () => {
    expect(panelIdFromHash("connections", KNOWN)).toBe("connections");
  });

  it("falls back to the first panel when there is no hash", () => {
    expect(panelIdFromHash("", KNOWN)).toBe("home");
    expect(panelIdFromHash("#", KNOWN)).toBe("home");
  });

  it("falls back rather than failing on a hash naming nothing", () => {
    // Typing nonsense into a window's address is not an error condition.
    expect(panelIdFromHash("#nope", KNOWN)).toBe("home");
  });

  it("ignores sub-state a panel may keep after the id", () => {
    expect(panelIdFromHash("#connections?edit=3", KNOWN)).toBe("connections");
    expect(panelIdFromHash("#connections/new", KNOWN)).toBe("connections");
  });

  it("is not case-sensitive", () => {
    expect(panelIdFromHash("#Connections", KNOWN)).toBe("connections");
  });

  it("refuses to resolve anything when there are no panels", () => {
    expect(() => panelIdFromHash("#home", [])).toThrow(/no panels/);
  });
});

describe("hashForPanel", () => {
  it("round-trips with panelIdFromHash", () => {
    for (const id of KNOWN) {
      expect(panelIdFromHash(hashForPanel(id), KNOWN)).toBe(id);
    }
  });
});
