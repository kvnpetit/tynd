import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { vault } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-vault client", () => {
  test("exists", async () => {
    await vault.exists("/v");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "vault", method: "exists", args: { path: "/v" } },
    ]);
  });

  test("write sends strings as-is", async () => {
    await vault.write("/v", "pw", "hi");
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "vault",
        method: "write",
        args: { path: "/v", password: "pw", data: "hi" },
      },
    ]);
  });

  test("write base64-encodes bytes", async () => {
    await vault.write("/v", "pw", new Uint8Array([104, 105]));
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "vault",
        method: "write",
        args: { path: "/v", password: "pw", data: expect.any(String) },
      },
    ]);
  });
});
