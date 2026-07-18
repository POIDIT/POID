/**
 * The Resolver, offline: the network is a fake, the tarball is crafted in
 * the test — what is under test is the contract: no download without a plan,
 * no plan under Tier 3, integrity always verified, and a consented download
 * ends as a project-local bundle the app build can alias.
 */

import { gzipSync } from "node:zlib";
import * as esbuildNative from "esbuild";
import { describe, expect, it, vi } from "vitest";
import { bundle, type EsbuildApi } from "./engine.js";
import { downloadAndBundle, IntegrityError, OfflinePolicyError, planDownload } from "./resolver.js";
import { gunzip, untar } from "./tar.js";

const engine = esbuildNative as unknown as EsbuildApi;
const encoder = new TextEncoder();

/** Builds a one-block ustar entry. */
function tarEntry(name: string, content: Uint8Array): Buffer {
  const header = Buffer.alloc(512);
  header.write(name, 0, "utf8");
  header.write("0000644", 100);
  header.write("0000000", 108);
  header.write("0000000", 116);
  header.write(`${content.length.toString(8).padStart(11, "0")} `, 124);
  header.write("00000000000 ", 136);
  header.write("        ", 148); // checksum spaces while summing
  header.write("0", 156);
  header.write("ustar", 257, "utf8");
  header.write("00", 263);
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148);
  const padded = Buffer.alloc(Math.ceil(content.length / 512) * 512);
  padded.set(content);
  return Buffer.concat([header, padded]);
}

function makeTarball(files: Record<string, string>): Buffer {
  const blocks = Object.entries(files).map(([name, text]) => tarEntry(name, encoder.encode(text)));
  blocks.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(blocks), { level: 9 });
}

async function integrityOf(bytes: Buffer): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-512", new Uint8Array(bytes).slice());
  return `sha512-${Buffer.from(digest).toString("base64")}`;
}

const TONE_TARBALL = makeTarball({
  "package/package.json": '{ "name": "tone", "version": "9.9.9", "main": "index.js" }',
  "package/index.js": 'module.exports = { note: () => "A4" };',
});

function fakeFetch(tarball: Buffer, integrity: string) {
  return vi.fn(async (url: string) => {
    if (url.endsWith("/latest")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          version: "9.9.9",
          dist: {
            tarball: "https://registry.npmjs.org/tone/-/tone-9.9.9.tgz",
            integrity,
            unpackedSize: 1234,
          },
        }),
        arrayBuffer: async () => new ArrayBuffer(0),
      };
    }
    return {
      ok: true,
      status: 200,
      json: async () => ({}),
      arrayBuffer: async (): Promise<ArrayBuffer> => new Uint8Array(tarball).slice().buffer,
    };
  });
}

describe("tar reader", () => {
  it("round-trips the crafted tarball", async () => {
    const entries = untar(await gunzip(new Uint8Array(TONE_TARBALL)));
    expect(entries.map((e) => e.path)).toEqual(["package/package.json", "package/index.js"]);
  });
});

describe("resolver", () => {
  it("Tier 3 refuses to even plan, with a human explanation", async () => {
    const fetchSpy = vi.fn();
    await expect(planDownload("tone", { offline: true }, fetchSpy)).rejects.toThrow(
      OfflinePolicyError,
    );
    await expect(planDownload("tone", { offline: true }, fetchSpy)).rejects.toThrow(
      /Standard Library/,
    );
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("a plan describes the download without performing it", async () => {
    const integrity = await integrityOf(TONE_TARBALL);
    const fetchSpy = fakeFetch(TONE_TARBALL, integrity);
    const plan = await planDownload("tone", { offline: false }, fetchSpy);
    expect(plan).toMatchObject({
      specifier: "tone",
      pkg: "tone",
      version: "9.9.9",
      sizeBytes: 1234,
    });
    expect(fetchSpy).toHaveBeenCalledTimes(1); // metadata only, no tarball
  });

  it("rejects a tampered tarball", async () => {
    const integrity = await integrityOf(TONE_TARBALL);
    const tampered = Buffer.from(TONE_TARBALL);
    const last = tampered.at(-1) ?? 0;
    tampered[tampered.length - 1] = last ^ 0xff;
    const plan = await planDownload("tone", { offline: false }, fakeFetch(TONE_TARBALL, integrity));
    await expect(downloadAndBundle(plan, engine, fakeFetch(tampered, integrity))).rejects.toThrow(
      IntegrityError,
    );
  });

  it("a consented download becomes a project-local bundle the app can use", async () => {
    const integrity = await integrityOf(TONE_TARBALL);
    const fetchSpy = fakeFetch(TONE_TARBALL, integrity);
    const plan = await planDownload("tone", { offline: false }, fetchSpy);
    const dep = await downloadAndBundle(plan, engine, fetchSpy);
    expect(dep.file).toBe("vendor/tone.js");
    expect(dep.record).toBe("tone@9.9.9");

    // The recipient story: the app builds against the stored file, offline.
    const files = new Map<string, Uint8Array>([
      ["src/main.ts", encoder.encode('import t from "tone"; console.log(t.note());')],
      [dep.file, dep.content],
    ]);
    const out = await bundle(engine, {
      files,
      entry: "src/main.ts",
      aliases: { tone: dep.file },
    });
    expect(new TextDecoder().decode(out.js)).toContain("A4");
  });
});
