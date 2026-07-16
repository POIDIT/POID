/**
 * The execution watchdog (SPEC §9.4). The host pings the sandbox on an
 * interval; a live application's event loop delivers the message and the SDK
 * auto-answers `pong`. An app stuck in an infinite loop cannot answer, so a
 * missed deadline marks it unresponsive and the reader can terminate it.
 *
 * Termination itself is host-side (tearing down the iframe), so the user's
 * "Stop this application" control always works regardless of app state.
 */

/** Timer primitives, injectable so the watchdog is testable with fake time. */
export interface WatchdogClock {
  now(): number;
  setInterval(fn: () => void, ms: number): number;
  clearInterval(handle: number): void;
}

const realClock: WatchdogClock = {
  now: () => Date.now(),
  setInterval: (fn, ms) => setInterval(fn, ms) as unknown as number,
  clearInterval: (h) => clearInterval(h),
};

export interface WatchdogOptions {
  /** How often to ping, ms (default 1000). */
  intervalMs?: number;
  /** How long without a pong before the app is unresponsive, ms (default 5000). */
  timeoutMs?: number;
  clock?: WatchdogClock;
}

/** Drives liveness for one sandboxed window. */
export class Watchdog {
  private readonly intervalMs: number;
  private readonly timeoutMs: number;
  private readonly clock: WatchdogClock;
  private readonly ping: (seq: number) => void;
  private readonly onUnresponsive: () => void;

  private handle: number | null = null;
  private seq = 0;
  private lastPong = 0;
  private started = 0;
  private tripped = false;

  constructor(
    ping: (seq: number) => void,
    onUnresponsive: () => void,
    options: WatchdogOptions = {},
  ) {
    this.ping = ping;
    this.onUnresponsive = onUnresponsive;
    this.intervalMs = options.intervalMs ?? 1000;
    this.timeoutMs = options.timeoutMs ?? 5000;
    this.clock = options.clock ?? realClock;
  }

  /** Begins pinging. */
  start(): void {
    if (this.handle !== null) return;
    this.started = this.clock.now();
    this.lastPong = this.started;
    this.handle = this.clock.setInterval(() => this.tick(), this.intervalMs);
  }

  /** Stops pinging (on terminate or close). */
  stop(): void {
    if (this.handle !== null) {
      this.clock.clearInterval(this.handle);
      this.handle = null;
    }
  }

  /** Records a `pong` from the sandbox. */
  onPong(seq: number): void {
    if (seq <= this.seq) this.lastPong = this.clock.now();
  }

  /** True once the app has missed the deadline. */
  get isUnresponsive(): boolean {
    return this.tripped;
  }

  private tick(): void {
    const now = this.clock.now();
    if (!this.tripped && now - this.lastPong > this.timeoutMs) {
      this.tripped = true;
      this.stop();
      this.onUnresponsive();
      return;
    }
    this.seq += 1;
    this.ping(this.seq);
  }
}
