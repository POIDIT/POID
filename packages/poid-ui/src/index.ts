/**
 * `@poid/ui` — the reader chrome, rendered outside the sandbox.
 *
 * The consent screen (SECURITY §5), the connection-choice prompt (SPEC
 * §7.2.3), title bar, dirty indicator, storage badge and "Stop this
 * application" control live in the host document. The application cannot
 * style, cover, suppress or trigger any of them.
 */

export {
  type BindingCandidate,
  type BindingChoice,
  type BindingHandlers,
  type BindingModel,
  renderBindingChoice,
} from "./binding.js";
export { type ConsentManifest, type ConsentModel, consentModel } from "./consent.js";
export {
  type ChromeHandle,
  type ConsentHandlers,
  el,
  renderChrome,
  renderConsent,
} from "./dom.js";
