import { afterEach, describe, expect, test } from "bun:test";
import { capabilities } from "./capabilities";
import { resolveHostCall, setHostCallTransport } from "./host";

interface Frame {
  id: string;
  api: string;
  method: string;
  args: unknown;
}

afterEach(() => setHostCallTransport(null));

function capture(result: unknown = null): Frame[] {
  const frames: Frame[] = [];
  setHostCallTransport((frame) => {
    frames.push(frame);
    queueMicrotask(() => resolveHostCall(frame.id, true, result));
  });
  return frames;
}

describe("capabilities (backend scope grants)", () => {
  test("sends every grant method to the security API", async () => {
    const frames = capture();
    await capabilities.allowPath("/data", { recursive: true, write: true });
    await capabilities.allowPath("/a.txt");
    await capabilities.revokePath("/data");
    await capabilities.allowUrl("https://*.example.com/**");
    await capabilities.revokeUrl("https://*.example.com/**");
    await capabilities.list();
    expect(frames.map(({ api, method, args }) => ({ api, method, args }))).toEqual([
      {
        api: "security",
        method: "allowPath",
        args: { path: "/data", recursive: true, write: true },
      },
      { api: "security", method: "allowPath", args: { path: "/a.txt" } },
      { api: "security", method: "revokePath", args: { path: "/data" } },
      { api: "security", method: "allowUrl", args: { pattern: "https://*.example.com/**" } },
      { api: "security", method: "revokeUrl", args: { pattern: "https://*.example.com/**" } },
      { api: "security", method: "listGrants", args: null },
    ]);
  });

  test("returns the host's answer", async () => {
    const grants = { paths: [], urls: ["https://example.com/**"] };
    capture(grants);
    expect(await capabilities.list()).toEqual(grants);
  });

  test("is unavailable before app.start()", async () => {
    await expect(capabilities.allowPath("/data")).rejects.toThrow("trusted Bun backend");
  });
});
