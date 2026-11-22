import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { sidecar } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-sidecar client", () => {
  test("path", async () => {
    await sidecar.path("worker");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "sidecar", method: "path", args: { name: "worker" } },
    ]);
  });

  test("list", async () => {
    await sidecar.list();
    expect(bridge.calls).toEqual([{ kind: "os", api: "sidecar", method: "list", args: null }]);
  });
});
