import { describe, expect, it, vi } from "vitest";
import { EngineIntegrityError, engineSatisfies, verifyEngine } from "./engines.js";

const encoder = new TextEncoder();

async function digestOf(text: string): Promise<string> {
  const bytes = encoder.encode(text);
  const digest = await crypto.subtle.digest("SHA-256", bytes.slice().buffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

describe("verifyEngine", () => {
  it("accepts an engine whose every file matches its pin", async () => {
    const manifest = {
      name: "pyodide",
      version: "0.26.4",
      compatible: ">=0.26 <0.27",
      files: { "pyodide.js": await digestOf("engine code") },
    };
    const fetchImpl = vi.fn(async () => ({
      ok: true,
      arrayBuffer: async (): Promise<ArrayBuffer> => encoder.encode("engine code").slice().buffer,
    }));
    await expect(verifyEngine("/engine/", manifest, fetchImpl)).resolves.toBeUndefined();
    expect(fetchImpl).toHaveBeenCalledWith("/engine/pyodide.js");
  });

  it("refuses a tampered engine before anything runs", async () => {
    const manifest = {
      name: "pyodide",
      version: "0.26.4",
      compatible: ">=0.26 <0.27",
      files: { "pyodide.js": await digestOf("engine code") },
    };
    const fetchImpl = async () => ({
      ok: true,
      arrayBuffer: async (): Promise<ArrayBuffer> => encoder.encode("tampered code").slice().buffer,
    });
    await expect(verifyEngine("/engine/", manifest, fetchImpl)).rejects.toThrow(
      EngineIntegrityError,
    );
  });
});

describe("engineSatisfies", () => {
  it("matches the ranges manifests actually write", () => {
    expect(engineSatisfies("0.26.4", ">=0.26 <0.27")).toBe(true);
    expect(engineSatisfies("0.26.4", ">=0.26 <0.28")).toBe(true);
    expect(engineSatisfies("0.27.0", ">=0.26 <0.27")).toBe(false);
    expect(engineSatisfies("0.25.1", ">=0.26 <0.28")).toBe(false);
    expect(engineSatisfies("0.26.4", "~0.26.0")).toBe(true);
    expect(engineSatisfies("0.26.4", "0.26")).toBe(true);
    expect(engineSatisfies("1.2.3", "^1.1")).toBe(true);
  });

  it("treats garbage ranges as mismatches, never silent passes", () => {
    expect(engineSatisfies("0.26.4", "whatever")).toBe(false);
    expect(engineSatisfies("not-a-version", ">=0.26")).toBe(false);
    expect(engineSatisfies("0.26.4", "")).toBe(false);
  });
});
