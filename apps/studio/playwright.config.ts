import { defineConfig } from "@playwright/test";

/**
 * The desktop tier: these tests drive the built Studio binary over CDP rather
 * than a page in a browser, so there is no `webServer`, no browser project and
 * no `baseURL` — `e2e/studio-app.ts` launches the application and hands back
 * its windows as pages.
 *
 * `workers: 1` is not a performance choice. Studio holds a single-instance
 * lock, so two concurrent launches would collapse into one process and the
 * tests would drive each other's windows.
 */
export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.spec.ts",
  fullyParallel: false,
  workers: 1,
  // Launching a desktop application, opening a webview and cold-starting a
  // WASM validator costs far more than loading a page.
  timeout: 120_000,
  reporter: process.env.CI ? "line" : "list",
});
