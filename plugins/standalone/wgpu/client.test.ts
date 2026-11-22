import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { wgpu } from "./client";

// The core bridge reads `window.__tynd__`; in a webview `window` is `globalThis`.
const previousWindow = (globalThis as { window?: unknown }).window;
beforeAll(() => {
  Object.assign(globalThis, { window: globalThis });
});
afterAll(() => {
  Object.assign(globalThis, { window: previousWindow });
});

type Call = { target: string; args: unknown };

const RAW_HANDLE = {
  id: 7,
  width: 640,
  height: 480,
  backend: "dx12",
  adapter: { vendor: "0x10de", device: "GPU", driver: "1.0" },
};

let calls: Call[];
let responses: Record<string, unknown>;
const globals = globalThis as { __tynd__?: unknown };
const original = globals.__tynd__;

beforeEach(() => {
  calls = [];
  responses = { "wgpu:createSurface": RAW_HANDLE, "wgpu:close": { closed: true } };
  globals.__tynd__ = {
    os_call: async (_ns: string, target: string, args: unknown) => {
      calls.push({ target, args });
      return responses[target] ?? null;
    },
  };
});

afterEach(() => {
  globals.__tynd__ = original;
});

describe("wgpu client", () => {
  test("createSurface fills defaults mirroring the Rust SurfaceOpts", async () => {
    const surface = await wgpu.createSurface();
    expect(calls[0]).toEqual({
      target: "wgpu:createSurface",
      args: {
        opts: {
          title: null,
          width: 1280,
          height: 720,
          backend: "auto",
          powerPreference: "high-performance",
          presentMode: "fifo",
          format: "bgra8unorm",
        },
      },
    });
    expect(surface.id).toBe(7);
    expect(surface.width).toBe(640);
    expect(surface.backend).toBe("dx12");
  });

  test("createSurface keeps caller overrides", async () => {
    await wgpu.createSurface({ width: 800, height: 600, backend: "vulkan", title: "demo" });
    const sent = ((calls[0]?.args ?? {}) as { opts: Record<string, unknown> }).opts;
    expect(sent.width).toBe(800);
    expect(sent.height).toBe(600);
    expect(sent.backend).toBe("vulkan");
    expect(sent.title).toBe("demo");
  });

  test("resize forwards the id and updates the handle size", async () => {
    const surface = await wgpu.createSurface();
    await surface.resize(1024, 768);
    expect(calls.at(-1)).toEqual({
      target: "wgpu:resize",
      args: { id: 7, width: 1024, height: 768 },
    });
    expect(surface.width).toBe(1024);
    expect(surface.height).toBe(768);
  });

  test("a rejected resize leaves the cached size untouched", async () => {
    const surface = await wgpu.createSurface();
    globals.__tynd__ = {
      os_call: async () => {
        throw new Error("width 100000 exceeds the maximum of 8192");
      },
    };
    await expect(surface.resize(100_000, 100_000)).rejects.toThrow("exceeds the maximum");
    expect(surface.width).toBe(640);
  });

  test("close is idempotent and hits the native side once", async () => {
    const surface = await wgpu.createSurface();
    await surface.close();
    await surface.close();
    expect(calls.filter((c) => c.target === "wgpu:close")).toHaveLength(1);
  });
});

describe("wgpu client list", () => {
  test("list reuses the wrapper of a known surface and creates none twice", async () => {
    const surface = await wgpu.createSurface();
    responses["wgpu:list"] = [RAW_HANDLE];
    const [first] = await wgpu.list();
    const [second] = await wgpu.list();
    expect(first).toBe(surface);
    expect(second).toBe(surface);
  });

  test("list forgets surfaces that are no longer open", async () => {
    const surface = await wgpu.createSurface();
    responses["wgpu:list"] = [];
    expect(await wgpu.list()).toEqual([]);
    responses["wgpu:list"] = [RAW_HANDLE];
    const [again] = await wgpu.list();
    expect(again).not.toBe(surface);
  });
});
