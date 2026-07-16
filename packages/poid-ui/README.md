# @poid/ui

The reader chrome, rendered **outside** the sandbox: the consent screen
(SECURITY §5), title bar, dirty indicator, storage badge, and "Stop this
application" control. Framework-free plain DOM, zero runtime dependencies.

Because these live in the host document, the application has no reference to
them and no channel that can reach them — the consent screen and the Stop
button cannot be defeated from inside the container.

```
pnpm --filter @poid/ui build
pnpm --filter @poid/ui test
```
