import { defineConfig, devices } from "@playwright/test";

/**
 * The browser tier for the Web Reader. A real Chromium proves what jsdom
 * cannot: CSP enforcement, sandboxed opaque origins, the service worker, and
 * the M05 Definition of Done — zero network requests after the initial page
 * load, and full function offline.
 */
const PORT = 5179;

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
        // Same isolation posture as the desktop reader tests (see
        // packages/poid-host/playwright.config.ts).
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
    timeout: 120_000,
  },
});
