import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { cli } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-cli client", () => {
  test("raw", async () => {
    await cli.raw();
    expect(bridge.calls).toEqual([{ kind: "os", api: "cli", method: "raw", args: {} }]);
  });
});
