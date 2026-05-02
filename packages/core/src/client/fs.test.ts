import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { fs } from "./fs";

type FetchCall = { url: string; method: string; body: Uint8Array | null };

const originalFetch = globalThis.fetch;

function mountShim(handler: (c: FetchCall) => { status: number; body: Uint8Array }): FetchCall[] {
  const fetches: FetchCall[] = [];
  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call() {
        return Promise.resolve(null);
      },
      os_on() {
        return () => undefined;
      },
      call() {
        return Promise.resolve(null);
      },
      on() {
        return () => undefined;
      },
      off() {
        return undefined;
      },
    },
  };
  globalThis.fetch = ((url: string, init?: RequestInit) => {
    const bodyInit = init?.body;
    const body =
      bodyInit instanceof Uint8Array
        ? bodyInit
        : bodyInit instanceof ArrayBuffer
          ? new Uint8Array(bodyInit)
          : null;
    const call: FetchCall = { url, method: init?.method ?? "GET", body };
    fetches.push(call);
    const res = handler(call);
    return Promise.resolve(new Response(res.body as BodyInit, { status: res.status }));
  }) as typeof fetch;
  return fetches;
}

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("fs binary (tynd-bin scheme)", () => {
  let fetches: FetchCall[];
  beforeEach(() => {
    fetches = mountShim(() => ({ status: 200, body: new Uint8Array([1, 2, 3, 4]) }));
  });

  test("readBinary issues a GET and returns bytes", async () => {
    const bytes = await fs.readBinary("/tmp/x.bin");
    expect(bytes).toEqual(new Uint8Array([1, 2, 3, 4]));
    expect(fetches[0]?.method).toBe("GET");
    expect(fetches[0]?.url).toContain("tynd-bin://localhost/fs/readBinary");
    expect(fetches[0]?.url).toContain("path=%2Ftmp%2Fx.bin");
  });

  test("writeBinary POSTs the raw body", async () => {
    const payload = new Uint8Array([9, 8, 7]);
    mountShim(() => ({ status: 204, body: new Uint8Array() }));
    await fs.writeBinary("/tmp/out.bin", payload);
  });

  test("writeBinary carries createDirs in the query", async () => {
    const fs2Fetches = mountShim(() => ({ status: 204, body: new Uint8Array() }));
    await fs.writeBinary("/tmp/nested/out.bin", new Uint8Array([1]), { createDirs: true });
    expect(fs2Fetches[0]?.url).toContain("createDirs=1");
  });

  test("non-2xx response rejects", async () => {
    mountShim(() => ({ status: 500, body: new TextEncoder().encode("disk full") }));
    await expect(fs.readBinary("/tmp/x")).rejects.toThrow(/disk full/);
  });
});

describe("FileHandle async dispose (TC39 `await using`)", () => {
  function mountOsShim(): { calls: Array<{ method: string; args: unknown }> } {
    const calls: Array<{ method: string; args: unknown }> = [];
    let nextId = 1;
    (globalThis as unknown as { window: unknown }).window = {
      __tynd__: {
        os_call(_api: string, method: string, args: unknown) {
          calls.push({ method, args });
          if (method === "open") return Promise.resolve({ id: nextId++ });
          if (method === "close") return Promise.resolve(true);
          return Promise.resolve(null);
        },
        os_on() {
          return () => undefined;
        },
        call() {
          return Promise.resolve(null);
        },
        on() {
          return () => undefined;
        },
        off() {
          return undefined;
        },
      },
    };
    return { calls };
  }

  test("`await using` triggers fs.close on scope exit", async () => {
    const { calls } = mountOsShim();
    {
      // biome-ignore lint/correctness/noUnusedVariables: handle is consumed by `using`
      await using h = await fs.open("/tmp/x.txt");
      expect(calls.find((c) => c.method === "open")).toBeDefined();
      expect(calls.find((c) => c.method === "close")).toBeUndefined();
    }
    // After the block exits, the dispose hook fires.
    expect(calls.find((c) => c.method === "close")).toBeDefined();
  });

  test("explicit close() then dispose is a no-op (idempotent)", async () => {
    const { calls } = mountOsShim();
    const h = await fs.open("/tmp/x.txt");
    await h.close();
    const closeCallsAfterClose = calls.filter((c) => c.method === "close").length;
    expect(closeCallsAfterClose).toBe(1);
    // Manually invoke dispose; should NOT issue a second close.
    await h[Symbol.asyncDispose]();
    const closeCallsAfterDispose = calls.filter((c) => c.method === "close").length;
    expect(closeCallsAfterDispose).toBe(1);
  });

  test("double close() returns false and skips the host call", async () => {
    const { calls } = mountOsShim();
    const h = await fs.open("/tmp/x.txt");
    expect(await h.close()).toBe(true);
    // Second close — host already dropped the id, we short-circuit.
    expect(await h.close()).toBe(false);
    const closeCalls = calls.filter((c) => c.method === "close").length;
    expect(closeCalls).toBe(1);
  });

  test("dispose runs even when an error is thrown inside the using block", async () => {
    const { calls } = mountOsShim();
    let threw = false;
    try {
      await using h = await fs.open("/tmp/x.txt");
      void h.id;
      throw new Error("boom");
    } catch {
      threw = true;
    }
    expect(threw).toBe(true);
    expect(calls.find((c) => c.method === "close")).toBeDefined();
  });
});
