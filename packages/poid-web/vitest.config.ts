import { defineConfig } from "vitest/config";

// Vitest owns the Node logic tier (src/**/*.test.ts). The Playwright browser
// tier lives in e2e/ and must not be collected here.
//
// Since M07 the shared reader logic (and its unit tests) lives in @poid/host;
// what remains in this package is browser-glue proven by the e2e tier, so an
// empty vitest run is a valid state, not a misconfiguration.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    passWithNoTests: true,
  },
});
