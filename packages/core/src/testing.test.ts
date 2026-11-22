import { afterEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "./testing";

let mock: TyndMock | undefined;
afterEach(() => {
  mock?.restore();
  mock = undefined;
});

function osCall(api: string, method: string, args: unknown): Promise<unknown> {
  const bridge = (globalThis as unknown as { window: Window }).window.__tynd__;
  if (!bridge) throw new Error("bridge not installed");
  return bridge.os_call(api, method, args);
}

describe("installTyndMock os calls", () => {
  test("an unhandled OS call rejects without osFallback", async () => {
    mock = installTyndMock();
    await expect(osCall("fs", "readText", { path: "/a" })).rejects.toThrow(
      'no OS handler for "fs.readText"',
    );
    expect(mock.calls).toEqual([
      { kind: "os", api: "fs", method: "readText", args: { path: "/a" } },
    ]);
  });

  test("osFallback answers unmatched calls, named handlers take precedence", async () => {
    mock = installTyndMock({
      os: { "fs.exists": () => true },
      osFallback: (api, method, args) => ({ api, method, args }),
    });
    expect(await osCall("fs", "exists", { path: "/a" })).toBe(true);
    expect(await osCall("fs", "mkdir", { path: "/d" })).toEqual({
      api: "fs",
      method: "mkdir",
      args: { path: "/d" },
    });
    expect(mock.calls.map((c) => c.kind)).toEqual(["os", "os"]);
  });
});
