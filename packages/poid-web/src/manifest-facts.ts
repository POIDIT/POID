/**
 * Extracts the facts the Web Reader needs from a manifest that `poid-core`
 * has **already validated** — parsing here is mechanical field access with
 * spec defaults (SPEC §3.1), never re-validation.
 */

import type { Grant, ManifestFacts } from "@poid/host";
import type { ConsentManifest } from "@poid/ui";

/** The manifest slice the Web Reader consumes (SPEC §3.1). */
export interface WebManifestFacts {
  type: "app" | "data" | "workspace";
  /** Display name; for `type: data` the referenced `app_id`. */
  name: string;
  version: string;
  author?: string;
  entry?: string;
  /** `instance.id`, `null` until first open (SPEC §6.3). */
  instanceId: string | null;
  /** `runtime.profile`, e.g. `web`, `web+python`. */
  profile: string;
  /** `runtime.engines` semver ranges, keyed by engine name. */
  engines: Record<string, string>;
  storageMode: "embedded" | "vault" | "connection";
  slots: boolean;
  /** `storage.protected` — data is encrypted at rest (SPEC §9.2). */
  protectedData: boolean;
  permissions: {
    network: string[];
    filesystem: "none" | "user-initiated";
    clipboard: boolean;
    print: boolean;
    notifications: boolean;
    mcp: string[];
  };
  /** Present for `type: data` (SPEC §11). */
  dataRef?: { appId: string; appVersion: string; schema: string };
}

function asRecord(v: unknown): Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v)
    ? (v as Record<string, unknown>)
    : {};
}

function asString(v: unknown, fallback: string): string {
  return typeof v === "string" ? v : fallback;
}

function asBool(v: unknown): boolean {
  return v === true;
}

function asStringArray(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
}

function asStringRecord(v: unknown): Record<string, string> {
  const rec = asRecord(v);
  const out: Record<string, string> = {};
  for (const [k, val] of Object.entries(rec)) {
    if (typeof val === "string") out[k] = val;
  }
  return out;
}

/** Reads the Web Reader's manifest facts out of the validated manifest JSON. */
export function extractFacts(manifestJson: string): WebManifestFacts {
  const m = asRecord(JSON.parse(manifestJson));
  const app = asRecord(m.app);
  const runtime = asRecord(m.runtime);
  const storage = asRecord(m.storage);
  const perms = asRecord(m.permissions);
  const instance = asRecord(m.instance);

  const type = m.type === "data" || m.type === "workspace" ? m.type : "app";
  const storageMode =
    storage.mode === "vault" || storage.mode === "connection" ? storage.mode : "embedded";

  const facts: WebManifestFacts = {
    type,
    name: asString(app.name, "Untitled"),
    version: asString(app.version, "0.0.0"),
    entry: typeof m.entry === "string" ? m.entry : undefined,
    instanceId: typeof instance.id === "string" ? instance.id : null,
    profile: asString(runtime.profile, "web"),
    engines: asStringRecord(runtime.engines),
    storageMode,
    slots: asBool(storage.slots),
    protectedData: asBool(storage.protected),
    permissions: {
      network: asStringArray(perms.network),
      filesystem: perms.filesystem === "user-initiated" ? "user-initiated" : "none",
      clipboard: asBool(perms.clipboard),
      print: asBool(perms.print),
      notifications: asBool(perms.notifications),
      mcp: asStringArray(perms.mcp),
    },
  };
  if (typeof app.author === "string") facts.author = app.author;

  if (type === "data") {
    const ref = asRecord(m.data_ref);
    facts.dataRef = {
      appId: asString(ref.app_id, "unknown application"),
      appVersion: asString(ref.app_version, "0.0.0"),
      schema: asString(ref.schema, "unknown"),
    };
    facts.name = facts.dataRef.appId;
  }
  return facts;
}

/** Shapes the facts for the consent screen (SPEC §9.1). */
export function consentManifestFrom(
  facts: WebManifestFacts,
  signature: "none" | "valid" | "invalid",
): ConsentManifest {
  const consent: ConsentManifest = {
    name: facts.name,
    version: facts.version,
    signature,
    permissions: facts.permissions,
  };
  if (facts.author !== undefined) consent.author = facts.author;
  return consent;
}

/** Shapes the facts for `capabilitiesFromGrant` (`@poid/host`). */
export function hostFacts(facts: WebManifestFacts): ManifestFacts {
  return {
    storageMode: facts.storageMode,
    storageSlots: facts.slots,
    profile: facts.profile,
    permissions: facts.permissions,
  };
}

/**
 * The grant produced by the M05 consent screen: a single Run/Cancel decision,
 * so Run approves everything the manifest requested — and nothing more
 * (`capabilitiesFromGrant` still intersects with the request). Per-permission
 * toggles are a later milestone. `ai` is never granted here: the web reader
 * has no Connections and holds no keys.
 */
export function runGrant(facts: WebManifestFacts): Grant {
  return {
    network: facts.permissions.network,
    filesystem: facts.permissions.filesystem === "user-initiated",
    clipboard: facts.permissions.clipboard,
    print: facts.permissions.print,
    mcp: facts.permissions.mcp,
    ai: false,
  };
}
