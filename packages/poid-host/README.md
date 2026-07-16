# @poid/host

The host side of the sandbox bridge and the **boundary broker** — the security
boundary between a POID application and the reader (SECURITY.md, SPEC §5.2, §7).

- Creates the sandboxed iframe (`sandbox="allow-scripts"`, never
  `allow-same-origin`), serves the container from an opaque origin, and applies
  the CSP (`connect-src 'none'` by default).
- The broker brokers every `window.poid` call with four executable invariants:
  scope is derived from the sending window (never the message); unknown method
  or field fails closed; no credential ever crosses the boundary; every call is
  rate-limited and quota-checked.
- Watchdog: an app that hangs its event loop stops answering pings and is
  marked unresponsive; the host-side "Stop this application" always works.

**Any change here requires a security review note in the PR (CONTRIBUTING.md).**

```
pnpm --filter @poid/host test    # Node logic tier (broker, guard, bridge, watchdog)
pnpm --filter @poid/host e2e      # Playwright tier: real-browser enforcement
```

The Playwright tier proves the browser-enforced boundaries (CSP blocks fetch,
iframe escape, opaque origin, watchdog kill). It needs a Chromium:
`pnpm exec playwright install chromium`.
