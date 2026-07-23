import { defineConfig } from "vitest/config";

// Vitest owns the Node logic tier (src/**/*.test.ts). The desktop tier lives
// in e2e/ and is driven by Playwright against the built binary; collecting it
// here would run Playwright specs with no application to attach to.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
  },
});
