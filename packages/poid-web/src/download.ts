/**
 * *Download updated file* (M05): the Web Reader cannot write back to the
 * original file, so saving is an explicit download of a freshly packed
 * container with the current state embedded as `data/store.json` (SPEC §6.2).
 */

import type { WebPoidHandle } from "./wasm-api.js";

/** Packs the container with `state` as its embedded `data/store.json`.
 * The JSON is pretty-printed: the store must stay human-readable (SPEC §6.2). */
export function updatedFileBytes(poid: WebPoidHandle, state: Record<string, unknown>): Uint8Array {
  const body = `${JSON.stringify(state, null, 2)}\n`;
  poid.setData(new TextEncoder().encode(body));
  return poid.toBytes();
}

/** Triggers a browser download of container bytes under `filename`. */
export function triggerDownload(doc: Document, bytes: Uint8Array, filename: string): void {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);
  const blob = new Blob([buffer], { type: "application/vnd.poid+zip" });
  const url = URL.createObjectURL(blob);
  const a = doc.createElement("a");
  a.href = url;
  a.download = filename;
  doc.body.append(a);
  a.click();
  a.remove();
  // Deferred so the click's navigation can still read the blob.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
