/**
 * The consent screen (SECURITY §5, SPEC §9.1). Rendered by the reader
 * **outside** the sandbox: the application cannot style it, cover it, suppress
 * it, or trigger it. Execution does not begin until the user chooses Run.
 *
 * Listing what the app does *not* request is deliberate — it makes the safe
 * case visibly safe, which is what builds trust in the format.
 */

/** The permission-relevant facts the consent screen summarises (SPEC §3.1). */
export interface ConsentManifest {
  name: string;
  version: string;
  author?: string;
  signature: "none" | "valid" | "invalid";
  permissions: {
    network: string[];
    filesystem: "none" | "user-initiated";
    clipboard: boolean;
    print: boolean;
    notifications: boolean;
    mcp: string[];
  };
}

/** A human-readable list of what the app requests and what it does not. */
export interface ConsentModel {
  requests: string[];
  notRequests: string[];
}

/** Computes the consent summary. Pure and framework-free, so it is tested
 * without a DOM; the rendering step consumes it. */
export function consentModel(manifest: ConsentManifest): ConsentModel {
  const p = manifest.permissions;
  const requests: string[] = [];
  const notRequests: string[] = [];

  const line = (requested: boolean, yes: string, no: string) => {
    (requested ? requests : notRequests).push(requested ? yes : no);
  };

  requests.push("Store data in this file");

  line(p.network.length > 0, `Internet access: ${p.network.join(", ")}`, "Internet access");
  line(p.filesystem === "user-initiated", "Open and save files you choose", "Access to your files");
  line(p.clipboard, "Use the clipboard", "Access to your clipboard");
  line(p.print, "Print", "Printing");
  line(p.notifications, "Show notifications", "Notifications");
  line(p.mcp.length > 0, `External tools: ${p.mcp.join(", ")}`, "External tool access");

  // Credentials are never granted to an application, so it is always listed as
  // something the app does not receive (SECURITY §5).
  notRequests.push("Access to your credentials");

  return { requests, notRequests };
}
