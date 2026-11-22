import { afterEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "../testing";
import { binOrigin } from "./_internal";
import { createFrontendEmitter } from "./frontend-emitter";
import { workers } from "./workers";

const g = globalThis as Record<string, unknown>;
let mock: TyndMock | undefined;
afterEach(() => {
  mock?.restore();
  mock = undefined;
});

const settle = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));

describe("frontend emitter", () => {
  test("a failing emit does not raise an unhandled rejection", async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      mock = installTyndMock({
        backend: {
          __tynd_fe_emit__: () => {
            throw new Error("INVALID_ARGS");
          },
        },
      });
      createFrontendEmitter<{ toast: string }>().emit("toast", "hi");
      await settle(20);
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });
});

describe("workers.spawn under the default CSP", () => {
  test("uses a blob: URL (allowed by worker-src) and revokes it on terminate", async () => {
    const urls: string[] = [];
    class FakeWorker {
      onmessage: unknown;
      onerror: unknown;
      onmessageerror: unknown;
      constructor(url: string) {
        urls.push(url);
      }
      postMessage(): void {
        /* unused */
      }
      terminate(): void {
        /* unused */
      }
    }
    const realWorker = g.Worker;
    const revoked: string[] = [];
    const realRevoke = URL.revokeObjectURL;
    g.Worker = FakeWorker;
    URL.revokeObjectURL = (u: string) => {
      revoked.push(u);
    };
    try {
      const handle = await workers.spawn("(input) => input");
      expect(urls).toHaveLength(1);
      expect(urls[0]).toStartWith("blob:");
      expect(urls[0]).not.toStartWith("data:");
      await handle.terminate();
      expect(revoked).toEqual(urls);
    } finally {
      g.Worker = realWorker;
      URL.revokeObjectURL = realRevoke;
    }
  });
});

describe("binOrigin", () => {
  test("uses the http .localhost form on Windows and Android, the custom scheme elsewhere", () => {
    expect(binOrigin("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/130")).toBe(
      "http://tynd-bin.localhost",
    );
    expect(binOrigin("Mozilla/5.0 (Linux; Android 14)")).toBe("http://tynd-bin.localhost");
    expect(binOrigin("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit")).toBe(
      "tynd-bin://localhost",
    );
    expect(binOrigin(undefined)).toBe("tynd-bin://localhost");
  });

  test("prefers the host-injected origin over the user-agent guess", () => {
    const g = globalThis as Record<string, unknown>;
    g.__TYND_BIN_ORIGIN__ = "http://tynd-bin.localhost";
    try {
      expect(binOrigin("MyApp/2.0")).toBe("http://tynd-bin.localhost");
      g.__TYND_BIN_ORIGIN__ = "tynd-bin://localhost";
      expect(binOrigin("Mozilla/5.0 (Windows NT 10.0)")).toBe("tynd-bin://localhost");
      g.__TYND_BIN_ORIGIN__ = 42;
      expect(binOrigin("Mozilla/5.0 (Windows NT 10.0)")).toBe("http://tynd-bin.localhost");
    } finally {
      Reflect.deleteProperty(g, "__TYND_BIN_ORIGIN__");
    }
  });
});
