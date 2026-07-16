import { defineConfig, devices } from "@playwright/test";

/**
 * The browser test tier for the security boundary. These tests need a **real**
 * browser: CSP, iframe sandboxing and opaque origins are enforced by the
 * engine, not by jsdom. Node/vitest covers the boundary logic; this covers the
 * enforcement.
 */
const PORT = 5178;

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  reporter: process.env.CI ? "line" : "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        // A hung application must not freeze the reader. Real readers rely on
        // process isolation: a sandboxed application frame runs out-of-process,
        // so its infinite loop cannot block the host's watchdog. Force it on so
        // the DoD is proven under the same guarantee production depends on.
        launchOptions: {
          args: ["--site-per-process", "--enable-features=IsolateSandboxedIframes"],
        },
      },
    },
  ],
  webServer: {
    command: "node e2e/server.mjs",
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    env: { PORT: String(PORT) },
    timeout: 60_000,
  },
});
