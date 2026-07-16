// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import type { ConsentManifest } from "./consent.js";
import { renderChrome, renderConsent } from "./dom.js";

const manifest: ConsentManifest = {
  name: "Kanban",
  version: "1.0.0",
  author: "Jane Doe",
  signature: "none",
  permissions: {
    network: [],
    filesystem: "none",
    clipboard: false,
    print: false,
    notifications: false,
    mcp: [],
  },
};

describe("consent screen", () => {
  it("shows requested and not-requested lists and only runs on Run", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const onRun = vi.fn();
    const onCancel = vi.fn();
    renderConsent(host, manifest, { onRun, onCancel });

    expect(host.textContent).toContain("This application requests:");
    expect(host.textContent).toContain("It does NOT request:");
    expect(host.textContent).toContain("Access to your credentials");

    // Nothing runs until the user clicks Run.
    expect(onRun).not.toHaveBeenCalled();
    host.querySelector<HTMLButtonElement>(".poid-consent__run")?.click();
    expect(onRun).toHaveBeenCalledOnce();
    expect(onCancel).not.toHaveBeenCalled();
    // The dialog tears itself down after a decision.
    expect(host.querySelector(".poid-consent")).toBeNull();
  });

  it("cancel does not run the app", () => {
    const host = document.createElement("div");
    const onRun = vi.fn();
    const onCancel = vi.fn();
    renderConsent(host, manifest, { onRun, onCancel });
    host.querySelector<HTMLButtonElement>(".poid-consent__cancel")?.click();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onRun).not.toHaveBeenCalled();
  });
});

describe("reader chrome", () => {
  it("renders the Stop button and toggles dirty and badge", () => {
    const host = document.createElement("div");
    const onStop = vi.fn();
    const chrome = renderChrome(host, { title: "Kanban", storageMode: "embedded" }, onStop);

    const stop = host.querySelector<HTMLButtonElement>(".poid-chrome__stop");
    expect(stop?.textContent).toBe("Stop this application");
    stop?.click();
    expect(onStop).toHaveBeenCalledOnce();

    const dot = host.querySelector<HTMLElement>(".poid-chrome__dirty");
    expect(dot?.style.visibility).toBe("hidden");
    chrome.setDirty(true);
    expect(dot?.style.visibility).toBe("visible");

    chrome.setBadge("vault");
    expect(host.querySelector(".poid-chrome__badge")?.textContent).toBe("vault");

    chrome.setUnresponsive(true);
    expect(
      host.querySelector(".poid-chrome")?.classList.contains("poid-chrome--unresponsive"),
    ).toBe(true);
  });
});
