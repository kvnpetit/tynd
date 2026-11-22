import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { websocket } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-websocket client", () => {
  test("list", async () => {
    await websocket.list();
    expect(bridge.calls).toEqual([{ kind: "os", api: "websocket", method: "list", args: null }]);
  });
});
