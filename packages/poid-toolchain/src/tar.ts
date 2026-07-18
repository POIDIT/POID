/**
 * Minimal ustar reader for npm tarballs — enough to extract a registry
 * package in the browser (the Resolver runs in Studio, where there is no
 * `tar` binary and no Node). Handles the shapes npm actually produces:
 * ustar headers, the prefix field, GNU long names and pax path overrides.
 */

/** One extracted file. */
export interface TarEntry {
  /** Path inside the archive, `/`-separated. */
  path: string;
  content: Uint8Array;
}

const decoder = new TextDecoder();

function field(block: Uint8Array, offset: number, length: number): string {
  const raw = block.subarray(offset, offset + length);
  const end = raw.indexOf(0);
  return decoder.decode(end === -1 ? raw : raw.subarray(0, end));
}

function octal(block: Uint8Array, offset: number, length: number): number {
  const text = field(block, offset, length).trim();
  return text === "" ? 0 : Number.parseInt(text, 8);
}

/** Parses a (already gunzipped) tar archive. */
export function untar(bytes: Uint8Array): TarEntry[] {
  const entries: TarEntry[] = [];
  let offset = 0;
  let longName: string | undefined;
  let paxPath: string | undefined;

  while (offset + 512 <= bytes.length) {
    const block = bytes.subarray(offset, offset + 512);
    offset += 512;
    if (block.every((b) => b === 0)) break;

    const size = octal(block, 124, 12);
    const type = String.fromCharCode(block[156] ?? 0);
    const data = bytes.subarray(offset, offset + size);
    offset += Math.ceil(size / 512) * 512;

    if (type === "L") {
      // GNU long name: the data is the next entry's path.
      longName = decoder.decode(data).replace(/\0+$/, "");
      continue;
    }
    if (type === "x" || type === "g") {
      // pax header: records like `NN path=value\n`.
      const text = decoder.decode(data);
      const match = /\d+ path=([^\n]+)\n/.exec(text);
      if (match?.[1]) paxPath = match[1];
      continue;
    }
    if (type !== "0" && type !== "\0") {
      longName = undefined;
      paxPath = undefined;
      continue; // directories, links — never packed content
    }

    const prefix = field(block, 345, 155);
    const short = field(block, 0, 100);
    const path = paxPath ?? longName ?? (prefix ? `${prefix}/${short}` : short);
    longName = undefined;
    paxPath = undefined;
    entries.push({ path, content: data.slice() });
  }
  return entries;
}

/** Gunzips with the Web-standard DecompressionStream (browser and Node). */
export async function gunzip(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes.slice().buffer])
    .stream()
    .pipeThrough(new DecompressionStream("gzip"));
  const out = new Uint8Array(await new Response(stream).arrayBuffer());
  return out;
}
