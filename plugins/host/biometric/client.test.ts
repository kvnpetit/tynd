import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { biometric } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-biometric client", () => {
  test("isAvailable", async () => {
    await biometric.isAvailable();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "biometric", method: "isAvailable", args: {} },
    ]);
  });

  test("authenticate", async () => {
    await biometric.authenticate("Unlock");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "biometric", method: "authenticate", args: { reason: "Unlock" } },
    ]);
  });
});
