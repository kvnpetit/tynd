import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { appleScript } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-apple-script client", () => {
  test("run forwards script and language", async () => {
    await appleScript.run("return 1");
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "appleScript",
        method: "run",
        args: { script: "return 1", language: "AppleScript" },
      },
    ]);
  });
});
