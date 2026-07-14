import { expect, it } from "vitest";
import { PACKAGE_NAME } from "./index.js";

it("exposes its package name", () => {
  expect(PACKAGE_NAME).toBe("@poid/web");
});
