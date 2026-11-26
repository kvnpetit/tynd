import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { compute } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-compute client", () => {
  test("randomBytes decodes the base64 payload", async () => {
    bridge.restore();
    bridge = installTyndMock({ osFallback: () => "AQID" });
    expect(await compute.randomBytes(3)).toEqual(new Uint8Array([1, 2, 3]));
    expect(bridge.calls).toEqual([
      { kind: "os", api: "compute", method: "randomBytes", args: { n: 3 } },
    ]);
  });
});
