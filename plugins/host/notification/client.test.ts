import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { notification } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-notification client", () => {
  test("send fills an empty body", async () => {
    await notification.send("Hi");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "send", args: { title: "Hi", body: "" } },
    ]);
  });

  test("cancel", async () => {
    await notification.cancel(3);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "cancel", args: { id: 3 } },
    ]);
  });

  test("checkPermission", async () => {
    await notification.checkPermission();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "checkPermission", args: null },
    ]);
  });
});
