import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { network } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-network client", () => {
  test("interfaces", async () => {
    await network.interfaces();
    expect(bridge.calls).toEqual([{ kind: "os", api: "network", method: "interfaces", args: {} }]);
  });
});
