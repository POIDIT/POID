/**
 * Derives the granted capability set from the manifest's requested permissions
 * and the user's consent. A capability is granted only when the manifest
 * declared it **and** the user approved it (SPEC §9.1) — the manifest is a
 * request, never a grant.
 */

import type { Capability } from "@poid/sdk";

/** The permission-relevant slice of a manifest (SPEC §3.1). */
export interface ManifestFacts {
  storageMode: "embedded" | "vault" | "connection";
  storageSlots: boolean;
  /** `runtime.profile`, e.g. `web`, `web+sql`, `web+python`. */
  profile: string;
  permissions: {
    network: string[];
    filesystem: "none" | "user-initiated";
    clipboard: boolean;
    print: boolean;
    notifications: boolean;
    mcp: string[];
  };
}

/** What the user actually approved on the consent screen (SPEC §9.1). */
export interface Grant {
  /** Approved network origins (subset of the requested allowlist). */
  network: string[];
  filesystem: boolean;
  clipboard: boolean;
  print: boolean;
  /** Approved MCP server ids. */
  mcp: string[];
  /** The user has configured an AI Connection this app may use. */
  ai: boolean;
}

/**
 * Computes the capabilities to expose. Core document capabilities (`app`,
 * `db.kv`, `db.docs`, `db.slots`, `ui`, `export`) are always present; network,
 * files, AI, MCP, clipboard and print appear only when requested and granted.
 */
export function capabilitiesFromGrant(manifest: ManifestFacts, grant: Grant): Capability[] {
  const caps: Capability[] = ["app", "db.kv", "db.docs", "db.slots", "ui", "export"];

  const profileParts = manifest.profile.split("+");
  if (profileParts.includes("sql") || manifest.storageMode === "connection") {
    caps.push("db.sql");
  }
  if (manifest.permissions.filesystem === "user-initiated" && grant.filesystem) {
    caps.push("files");
  }
  // Network requires both a non-empty requested allowlist and approved origins.
  if (manifest.permissions.network.length > 0 && grant.network.length > 0) {
    caps.push("net");
  }
  if (grant.ai) {
    caps.push("ai");
  }
  if (manifest.permissions.mcp.length > 0 && grant.mcp.length > 0) {
    caps.push("mcp");
  }
  if (manifest.permissions.clipboard && grant.clipboard) {
    caps.push("ui.clipboard");
  }
  if (manifest.permissions.print && grant.print) {
    caps.push("ui.print");
  }
  return caps;
}
