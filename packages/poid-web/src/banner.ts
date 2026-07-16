/**
 * The honest-limitations banner (M05): one discreet, dismissible line. Once
 * dismissed it stays dismissed — the milestone spec says "do not pester".
 */

const DISMISS_KEY = "poid-web:banner-dismissed";

function isDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

function remember(): void {
  try {
    localStorage.setItem(DISMISS_KEY, "1");
  } catch {
    // Private browsing: the banner simply reappears next visit.
  }
}

/** Mounts the banner into `container` unless previously dismissed. */
export function mountBanner(container: HTMLElement): void {
  if (isDismissed()) return;
  const doc = container.ownerDocument;

  const banner = doc.createElement("div");
  banner.className = "poid-banner";

  const text = doc.createElement("span");
  text.textContent =
    "You're using the web reader. Install POID Studio for double-click opening, sync, and connections.";

  const dismiss = doc.createElement("button");
  dismiss.type = "button";
  dismiss.className = "poid-banner__dismiss";
  dismiss.textContent = "Dismiss";
  dismiss.setAttribute("aria-label", "Dismiss this notice");
  dismiss.addEventListener("click", () => {
    remember();
    banner.remove();
  });

  banner.append(text, dismiss);
  container.append(banner);
}
