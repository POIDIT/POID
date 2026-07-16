/**
 * Human-readable explanations for container rejections. Errors are written
 * for a person, not a developer (CONVENTIONS: UX rules) — the technical
 * message from the core is shown alongside, smaller, for those who want it.
 *
 * Every normative registry code (`spec/errors.md`) has an entry; a unit test
 * parses the registry and fails the build if one is missing.
 */

/** One explanation per normative `POID-xxx` registry code. */
export const REGISTRY_EXPLANATIONS: Record<string, string> = {
  "POID-001":
    "This file doesn't identify itself as a POID container. It may be another kind of file renamed to .poid.",
  "POID-002":
    "The container's type marker is present but malformed, so the file can't be trusted as a POID.",
  "POID-003": "This file could not be read as a POID container at all.",
  "POID-004": "Part of the container is damaged and can't be read.",
  "POID-010": "The container is missing its manifest, which every POID must carry.",
  "POID-011": "The container's manifest is damaged and can't be read.",
  "POID-012": "The container's manifest breaks the rules of the POID format.",
  "POID-020":
    "This file contains a native program. A POID can never contain native programs — that is what makes POID files safe to open — so it was refused.",
  "POID-021": "This file contains a link entry, which POID files are not allowed to carry.",
  "POID-022": "A file inside this container tries to escape onto your computer. It was refused.",
  "POID-023":
    "This container expands to a suspiciously large amount of data — a likely “zip bomb”.",
  "POID-024": "This container's contents are larger than a POID is allowed to be.",
  "POID-025": "This container uses a compression method the POID format doesn't allow.",
  "POID-026": "Archives nested inside this container go deeper than the format allows.",
  "POID-027": "Two files inside this container share the same name.",
  "POID-028": "This container holds more entries than a POID may contain.",
  "POID-030": "The application's start page is missing from the container.",
  "POID-031":
    "The contents don't match the container's own integrity record — the file was modified or damaged after it was built.",
  "POID-032": "The container declares an application but carries no application files.",
  "POID-040":
    "This file claims to be pure data but contains program code. Data containers must be inert, so it was refused.",
  "POID-041": "This workspace container holds no applications.",
  "POID-050":
    "The publisher's signature doesn't match the contents — the file was changed after it was signed, or the signature is forged.",
  "POID-051": "The publisher's signature is damaged and can't be checked.",
};

/** Shown when a rejection has no registry code (environmental failures). */
export const FALLBACK_EXPLANATION =
  "This file could not be opened. It was refused before anything in it could run.";

/** The explanation for a registry code, with a safe fallback. */
export function explain(registry: string | undefined): string {
  return (registry && REGISTRY_EXPLANATIONS[registry]) || FALLBACK_EXPLANATION;
}
