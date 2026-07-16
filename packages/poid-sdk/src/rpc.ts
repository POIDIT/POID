/**
 * The request/response plumbing: one outbound `postMessage` per call, matched
 * to its reply by id, resolved or rejected as a Promise (RUNTIME-API §0).
 */

import { PoidError } from "./errors.js";
import { type EventMessage, type HostMessage, PROTOCOL } from "./protocol.js";

/** The minimal message channel the RPC client needs. Abstracted so the SDK is
 * testable without a browser: in the sandbox it is backed by `postMessage`. */
export interface Transport {
  /** Send a message to the host. */
  post(message: unknown): void;
  /** Register the single inbound-message handler. */
  onMessage(handler: (data: unknown) => void): void;
}

/** Narrows unknown inbound data to a host message for this protocol. */
function asHostMessage(data: unknown): HostMessage | undefined {
  if (typeof data !== "object" || data === null) return undefined;
  const m = data as { poid?: unknown };
  if (m.poid !== PROTOCOL) return undefined;
  return data as HostMessage;
}

/** Matches an inbound message to a pending call and dispatches lifecycle
 * events. Answers `ping` immediately so liveness reflects the app's event
 * loop, not its cooperation. */
export class RpcClient {
  private seq = 0;
  private readonly pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: unknown) => void }
  >();
  private readonly listeners = new Map<string, Set<(data: unknown) => void>>();

  constructor(private readonly transport: Transport) {
    transport.onMessage((data) => this.handle(data));
  }

  /** Issues a brokered call and resolves with its result. */
  call(method: string, params: unknown): Promise<unknown> {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.transport.post({ poid: PROTOCOL, id, method, params });
    });
  }

  /** Subscribes to a host lifecycle event (e.g. `suspend`). */
  on(event: string, handler: (data: unknown) => void): void {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(handler);
  }

  private handle(data: unknown): void {
    const message = asHostMessage(data);
    if (!message) return;

    if ("ping" in message) {
      this.transport.post({ poid: PROTOCOL, pong: message.ping });
      return;
    }

    if ("event" in message) {
      const evt = message as EventMessage;
      const set = this.listeners.get(evt.event);
      if (set) for (const handler of set) handler(evt.data);
      return;
    }

    if ("id" in message) {
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      if (message.ok) {
        entry.resolve(message.result);
      } else {
        entry.reject(new PoidError(message.error.code, message.error.message));
      }
    }
  }
}
