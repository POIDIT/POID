/**
 * The postMessage bridge. This is where **scope is bound to the window**: each
 * registered sandbox has a `sourceWindow`, and an inbound message is attributed
 * to a session only by matching `event.source` against those windows by
 * identity (SECURITY §4). A message from an unknown window is dropped; a
 * message that *claims* another identity in its body cannot change which
 * session it is attributed to, because attribution never reads the body.
 */

import { PROTOCOL } from "@poid/sdk";
import type { Broker } from "./broker.js";
import type { Watchdog } from "./watchdog.js";

/** A registered sandbox window. `sourceWindow` is compared by reference; `post`
 * delivers a message back to that window. */
interface Registration {
  sessionId: string;
  sourceWindow: unknown;
  post: (message: unknown) => void;
  watchdog?: Watchdog;
}

/** The minimal `MessageEvent` shape the bridge consumes. */
export interface IncomingMessage {
  source: unknown;
  data: unknown;
}

/** Routes messages between sandboxed windows and the broker. */
export class SandboxBridge {
  private readonly registrations: Registration[] = [];

  constructor(private readonly broker: Broker) {}

  /** Attaches to a host window's `message` events. */
  attach(target: {
    addEventListener(type: "message", listener: (ev: IncomingMessage) => void): void;
  }): void {
    target.addEventListener("message", (ev) => void this.handleMessage(ev));
  }

  /** Registers a sandbox window under a session id assigned by the host. */
  register(reg: Registration): void {
    this.registrations.push(reg);
  }

  /** Removes a sandbox window (on close/terminate). */
  unregister(sessionId: string): void {
    const idx = this.registrations.findIndex((r) => r.sessionId === sessionId);
    if (idx !== -1) this.registrations.splice(idx, 1);
  }

  /**
   * Handles one inbound message. Attribution is by window identity only; the
   * message body is never consulted to decide *which* session sent it.
   */
  async handleMessage(ev: IncomingMessage): Promise<void> {
    const data = ev.data as { poid?: unknown; pong?: unknown };
    if (!data || data.poid !== PROTOCOL) return;

    // Scope from the window: find the registration whose window sent this.
    const reg = this.registrations.find((r) => r.sourceWindow === ev.source);
    if (!reg) return; // unknown/foreign window → drop

    if (typeof data.pong === "number") {
      reg.watchdog?.onPong(data.pong);
      return;
    }

    const response = await this.broker.handle(reg.sessionId, data);
    if (response) reg.post(response);
  }
}
