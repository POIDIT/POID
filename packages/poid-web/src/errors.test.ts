import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { explain, FALLBACK_EXPLANATION, REGISTRY_EXPLANATIONS } from "./errors.js";

/** Every code that appears in the normative registry table. */
function registryCodes(): string[] {
  const md = readFileSync(
    fileURLToPath(new URL("../../../spec/errors.md", import.meta.url)),
    "utf8",
  );
  const codes = new Set<string>();
  for (const match of md.matchAll(/\| (POID-\d{3}) \|/g)) {
    const code = match[1];
    if (code) codes.add(code);
  }
  return [...codes].sort();
}

describe("registry explanations", () => {
  it("covers every code in spec/errors.md", () => {
    const codes = registryCodes();
    expect(codes.length).toBeGreaterThan(0);
    for (const code of codes) {
      expect(REGISTRY_EXPLANATIONS[code], `missing explanation for ${code}`).toBeTruthy();
    }
  });

  it("has no explanation for a code the registry does not define", () => {
    const codes = new Set(registryCodes());
    for (const code of Object.keys(REGISTRY_EXPLANATIONS)) {
      expect(codes.has(code), `stale explanation for ${code}`).toBe(true);
    }
  });

  it("explains unknown or missing codes with the fallback", () => {
    expect(explain(undefined)).toBe(FALLBACK_EXPLANATION);
    expect(explain("POID-999")).toBe(FALLBACK_EXPLANATION);
    expect(explain("POID-020")).toContain("native");
  });
});
