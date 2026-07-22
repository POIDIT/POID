/**
 * The two DOM helpers every hub panel needs. Deliberately small: the hub
 * builds its markup the same way `@poid/ui` builds the reader's, so there is
 * one idiom in the product rather than a framework in one window and elements
 * in the other.
 */

/** Creates an element, optionally with a class and text content. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

/** Creates a `<button type="button">`, which is what every button here is. */
export function button(className: string | undefined, text: string): HTMLButtonElement {
  const node = el("button", className, text);
  node.type = "button";
  return node;
}
