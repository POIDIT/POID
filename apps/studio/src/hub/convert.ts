/**
 * The Converter panel (M12.2): drop a folder or a file, get a `.poid`.
 *
 * One drag, a progress line, a place to save. Never a console (UX rule 1). The
 * conversion itself is `poid-convert`'s, reached over IPC — the same pipeline
 * the CLI runs — so a project converts identically here and on the command
 * line. This file only gathers the dropped files and shows what happened.
 *
 * Today it walks the no-build path: a folder of ready-to-run files (HTML, CSS,
 * JavaScript) or a single HTML document. A project that needs bundling is
 * reported honestly rather than half-converted; the build engine that handles
 * it is a downloaded runtime arriving next.
 */

import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { button, el } from "./dom.js";
import { type BuildRequest, buildInApp } from "./esbuild-build.js";
import type { Panel } from "./panels.js";

/** One file to convert: a container-relative path and its bytes. */
interface InputFile {
  rel: string;
  bytes: Uint8Array;
}

/** What `convert_prepare` returns: a finished POID, or a build request. */
interface ConvertPrep extends BuildRequest {
  needsBuild: boolean;
  kind: string;
  poidBase64: string | null;
  fileCount: number;
  byteLen: number;
  token: string | null;
}

/** What `convert_finish` returns. */
interface ConvertOutcome {
  poidBase64: string;
  fileCount: number;
  byteLen: number;
}

/** Base64 of a byte array, chunked so a large file does not overflow the call
 * stack of `String.fromCharCode(...spread)`. */
function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/** Strips the single common leading directory a folder drop or a zip adds, so
 * `my-app/index.html` becomes `index.html`. Only strips when *every* file
 * shares the same first segment — otherwise the paths are already at the root. */
function stripCommonRoot(files: InputFile[]): InputFile[] {
  if (files.length === 0) return files;
  const first = files[0]?.rel.split("/")[0] ?? "";
  if (!first) return files;
  const shared = files.every((f) => f.rel === first || f.rel.startsWith(`${first}/`));
  if (!shared || files.some((f) => f.rel === first)) return files;
  return files.map((f) => ({ ...f, rel: f.rel.slice(first.length + 1) }));
}

/** Reads a `FileSystemEntry` tree (a dragged folder) into flat input files. */
async function readEntry(entry: FileSystemEntry, prefix: string): Promise<InputFile[]> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) =>
      (entry as FileSystemFileEntry).file(resolve, reject),
    );
    const bytes = new Uint8Array(await file.arrayBuffer());
    return [{ rel: `${prefix}${entry.name}`, bytes }];
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  const entries = await new Promise<FileSystemEntry[]>((resolve, reject) =>
    reader.readEntries(resolve, reject),
  );
  const nested = await Promise.all(entries.map((e) => readEntry(e, `${prefix}${entry.name}/`)));
  return nested.flat();
}

/** Reads dropped items (files and folders) into flat input files. */
async function readDrop(items: DataTransferItemList): Promise<InputFile[]> {
  const roots: FileSystemEntry[] = [];
  for (const item of Array.from(items)) {
    const entry = item.webkitGetAsEntry?.();
    if (entry) roots.push(entry);
  }
  const all = await Promise.all(roots.map((e) => readEntry(e, "")));
  return all.flat();
}

/** Reads an `<input type=file>` selection (a folder pick uses
 * `webkitRelativePath`). */
async function readInput(fileList: FileList): Promise<InputFile[]> {
  const out: InputFile[] = [];
  for (const file of Array.from(fileList)) {
    const rel = file.webkitRelativePath || file.name;
    out.push({ rel, bytes: new Uint8Array(await file.arrayBuffer()) });
  }
  return out;
}

/** A display name from the dropped files: the common root folder, else the one
 * file's stem, else "app". */
function nameFrom(files: InputFile[]): string {
  const first = files[0]?.rel ?? "";
  const root = first.includes("/") ? first.split("/")[0] : first.replace(/\.[^.]+$/, "");
  return root || "app";
}

export const convertPanel: Panel = {
  id: "convert",
  label: "Convert",
  title: "Convert to a POID",
  blurb:
    "Drop a folder or a file — a web app, or a single HTML page — and Studio turns it into a " +
    ".poid you can open, keep and send. Nothing is uploaded; the whole thing happens on this " +
    "computer.",

  mount(container) {
    const drop = el("div", "convert__drop");
    drop.append(
      el("p", "convert__hint", "Drop a folder or a file here"),
      el("p", "convert__sub", "or"),
    );

    const pickFolder = button("poid-drop__pick", "Choose a folder…");
    const pickFile = button(undefined, "Choose a file…");
    const picks = el("div", "convert__picks");
    picks.append(pickFolder, pickFile);
    drop.append(picks);

    // Hidden native inputs: one for a whole folder, one for single files.
    const folderInput = el("input", "convert__file") as HTMLInputElement;
    folderInput.type = "file";
    folderInput.webkitdirectory = true;
    const fileInput = el("input", "convert__file") as HTMLInputElement;
    fileInput.id = "convert-file-input";
    fileInput.type = "file";
    fileInput.multiple = true;

    const status = el("p", "convert__status");
    status.setAttribute("aria-live", "polite");

    container.append(drop, folderInput, fileInput, status);

    let busy = false;

    function setStatus(text: string, tone: "" | "error" | "ok" = ""): void {
      status.textContent = text;
      status.className = `convert__status${tone ? ` convert__status--${tone}` : ""}`;
    }

    async function convert(files: InputFile[]): Promise<void> {
      if (busy) return;
      files = stripCommonRoot(files.filter((f) => f.bytes.length > 0 || !f.rel.endsWith("/")));
      if (files.length === 0) {
        setStatus("There were no files to convert.", "error");
        return;
      }
      busy = true;
      drop.classList.remove("convert__drop--over");
      const name = nameFrom(files);
      try {
        setStatus(`Reading ${files.length} file${files.length === 1 ? "" : "s"}…`);
        const prep = await invoke<ConvertPrep>("convert_prepare", {
          files: files.map((f) => ({ rel: f.rel, bytesBase64: toBase64(f.bytes) })),
          displayName: name,
        });

        // Either the no-build path already produced a POID, or we must build.
        let poidBase64: string;
        let fileCount: number;
        let byteLen: number;
        if (prep.needsBuild) {
          if (!prep.token) throw new Error("the converter did not return a build handle");
          const bundle = await buildInApp(prep, setStatus);
          setStatus("Packing…");
          const outcome = await invoke<ConvertOutcome>("convert_finish", {
            token: prep.token,
            jsBase64: bundle.jsBase64,
            cssBase64: bundle.cssBase64,
          });
          poidBase64 = outcome.poidBase64;
          fileCount = outcome.fileCount;
          byteLen = outcome.byteLen;
        } else {
          if (!prep.poidBase64) throw new Error("the converter produced nothing to save");
          poidBase64 = prep.poidBase64;
          fileCount = prep.fileCount;
          byteLen = prep.byteLen;
        }

        const destination = await save({
          defaultPath: `${name}.poid`,
          filters: [{ name: "POID Document", extensions: ["poid"] }],
        });
        if (!destination) {
          setStatus("Ready to save — choose a location when you are.");
          return;
        }

        await invoke("write_poid", { path: destination, poidBase64 });
        const kb = (byteLen / 1024).toFixed(1);
        setStatus(`Saved ${fileCount} files (${kb} KiB) to ${destination}`, "ok");
      } catch (e) {
        setStatus(typeof e === "string" ? e : `Conversion failed: ${String(e)}`, "error");
      } finally {
        busy = false;
      }
    }

    // Drag and drop.
    drop.addEventListener("dragover", (e) => {
      e.preventDefault();
      drop.classList.add("convert__drop--over");
    });
    drop.addEventListener("dragleave", () => drop.classList.remove("convert__drop--over"));
    drop.addEventListener("drop", (e) => {
      e.preventDefault();
      const transfer = e.dataTransfer;
      if (!transfer) return;
      void (async () => {
        const files = transfer.items.length
          ? await readDrop(transfer.items)
          : await readInput(transfer.files);
        await convert(files);
      })();
    });

    // Pickers.
    pickFolder.addEventListener("click", () => folderInput.click());
    pickFile.addEventListener("click", () => fileInput.click());
    folderInput.addEventListener("change", () => {
      if (folderInput.files) void readInput(folderInput.files).then(convert);
    });
    fileInput.addEventListener("change", () => {
      if (fileInput.files) void readInput(fileInput.files).then(convert);
    });

    return undefined;
  },
};
