import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { keyring } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-keyring client", () => {
  test("set with an entry object", async () => {
    await keyring.set({ service: "s", account: "a" }, "pw");
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "keyring",
        method: "set",
        args: { service: "s", account: "a", password: "pw" },
      },
    ]);
  });

  test("get and delete forward the entry", async () => {
    await keyring.get({ service: "s", account: "a" });
    await keyring.delete({ service: "s", account: "a" });
    expect(bridge.calls.map((c) => c.kind === "os" && `${c.api}.${c.method}`)).toEqual([
      "keyring.get",
      "keyring.delete",
    ]);
  });
});
