import { defineConfig } from "vitest/config";

// Vitest owns the Node logic tier (src/**/*.test.ts). The Playwright browser
// tier lives in e2e/ and must not be collected here.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
  },
});
