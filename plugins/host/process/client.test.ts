import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { process } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-process client", () => {
  test("exec renames args to argv", async () => {
    await process.exec("git", { args: ["status"] });
    expect(bridge.calls).toEqual([
      { kind: "os", api: "process", method: "exec", args: { cmd: "git", argv: ["status"] } },
    ]);
  });

  test("execShell", async () => {
    await process.execShell("ls | wc");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "process", method: "execShell", args: { cmd: "ls | wc" } },
    ]);
  });
});
