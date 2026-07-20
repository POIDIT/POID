/**
 * The Studio hub window. In M07 it is deliberately a shell: the converter,
 * editor, AI, library and connections are later milestones and appear here
 * only as inert tiles. What it *can* do is open a document — which creates a
 * separate Reader window, never a panel inside the hub (ARCHITECTURE §2).
 */

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

function byId(id: string): HTMLElement {
  const node = document.getElementById(id);
  if (!node) throw new Error(`studio hub is missing #${id}`);
  return node;
}

byId("open-poid").addEventListener("click", () => {
  void (async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "POID Document", extensions: ["poid"] }],
    });
    if (typeof picked === "string") {
      await invoke("open_document", { path: picked });
    }
  })();
});
