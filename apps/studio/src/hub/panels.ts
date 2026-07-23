/**
 * The hub is a shell with panels, not a page.
 *
 * M07 gave the hub one screen with everything stacked on it. M12 adds eight
 * more tools, so the shape has to change before the content does: each tool is
 * a panel that owns its own markup, and the shell only decides which one is on
 * screen. Nothing in `index.html` belongs to any particular tool any more.
 *
 * This module is the part with no DOM in it — the registry's shape and the
 * rules for turning a location hash into a panel — so it can be tested as
 * plain logic. The wiring lives in `hub-main.ts` and the panels themselves.
 */

/** One tool in the hub. */
export interface Panel {
  /** Stable id, and the location hash that selects it. */
  readonly id: string;
  /** What the navigation calls it. */
  readonly label: string;
  /** The panel's own heading. */
  readonly title: string;
  /** One line under the heading, written for a person, not a developer. */
  readonly blurb: string;
  /**
   * Renders the panel into an empty container, and returns whatever stops it
   * again — a panel that starts a file watcher or holds a connection open must
   * be able to stop when the user navigates away. `undefined` when a panel has
   * nothing to stop, which is most of them; returning it explicitly keeps the
   * contract one type rather than a union with `void`.
   */
  mount(container: HTMLElement): Teardown | undefined;
}

/** Stops whatever a panel started. */
export type Teardown = () => void;

/**
 * Resolves a location hash to a panel id.
 *
 * Unknown and absent hashes both fall back to the first panel rather than
 * showing an error: a hub that cannot be reached by typing the wrong thing
 * into it is not a hub. `known` must not be empty.
 */
export function panelIdFromHash(hash: string, known: readonly string[]): string {
  const first = known[0];
  if (first === undefined) throw new Error("the hub has no panels");

  // Tolerate "#id", "id", "#id?query" and "#id/deeper": later milestones will
  // want a panel to keep sub-state in the hash, and the shell should already
  // be indifferent to it.
  const candidate = hash.replace(/^#/, "").split(/[?/]/)[0]?.toLowerCase() ?? "";
  return known.includes(candidate) ? candidate : first;
}

/** The location hash that selects `id`. */
export function hashForPanel(id: string): string {
  return `#${id}`;
}
