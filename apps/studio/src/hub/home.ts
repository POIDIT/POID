/**
 * The panel the hub opens on: open a document.
 *
 * It stays deliberately thin. Opening a POID creates a Reader window, never a
 * view inside the hub (ARCHITECTURE §2) — so the hub's job here is to pick a
 * file and get out of the way. The Library panel will grow beside it in M12.4.
 */

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { el } from "./dom.js";
import type { Panel } from "./panels.js";

export const homePanel: Panel = {
  id: "home",
  label: "Open",
  title: "Open a POID",
  blurb:
    "A POID opens in its own document window — double-clicking a .poid file anywhere does the " +
    "same and never shows this hub.",

  mount(container) {
    const pick = el("button", "poid-drop__pick", "Open a POID…");
    pick.type = "button";
    pick.id = "open-poid";
    pick.addEventListener("click", () => {
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
    container.append(pick);
    return undefined;
  },
};
