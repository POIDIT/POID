/**
 * The document payload a Reader window receives from the Rust core over IPC
 * (`reader_bootstrap`). Mirrors `src-tauri/src/document.rs` — the two files
 * are the one wire contract, change them together.
 */

/** One container file, base64-encoded for the JSON IPC hop. */
export interface FileEntryDto {
  path: string;
  dataB64: string;
}

/** What the Rust side hands a Reader window. */
export type DocumentDto =
  | {
      kind: "rejected";
      /** Normative registry code (`POID-xxx`); null for environmental errors. */
      registry: string | null;
      /** Stable diagnostic code, e.g. `native-code`. */
      code: string;
      /** Technical detail from the core. */
      message: string;
      fileName: string;
    }
  | {
      kind: "loaded";
      fileName: string;
      /** Absolute path of the opened file. */
      path: string;
      /** The validated manifest, serialized by `poid-core`. */
      manifestJson: string;
      signature: "none" | "valid";
      files: FileEntryDto[];
    };

/** Decodes the IPC file list into the `Map` the reader mounts from. */
export function decodeFiles(entries: FileEntryDto[]): Map<string, Uint8Array> {
  const files = new Map<string, Uint8Array>();
  for (const entry of entries) {
    const bin = atob(entry.dataB64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) {
      bytes[i] = bin.charCodeAt(i);
    }
    files.set(entry.path, bytes);
  }
  return files;
}
