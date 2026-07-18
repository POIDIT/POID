import { defineConfig } from "vitest/config";

// Node logic tier only; the parity tests spawn real esbuild engines and can
// take a few seconds, hence the generous timeout.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    testTimeout: 30_000,
  },
});
