import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { fs } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-fs client", () => {
  test("readText", async () => {
    await fs.readText("/a.txt");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "fs", method: "readText", args: { path: "/a.txt" } },
    ]);
  });

  test("mkdir forwards options", async () => {
    await fs.mkdir("/d", { recursive: true });
    expect(bridge.calls).toEqual([
      { kind: "os", api: "fs", method: "mkdir", args: { path: "/d", recursive: true } },
    ]);
  });

  test("rename", async () => {
    await fs.rename("/a", "/b");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "fs", method: "rename", args: { from: "/a", to: "/b" } },
    ]);
  });

  test("trash", async () => {
    await fs.trash("/a");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "fs", method: "trash", args: { path: "/a" } },
    ]);
  });
});
