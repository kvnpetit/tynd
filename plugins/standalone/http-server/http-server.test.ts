import { describe, expect, test } from "bun:test";
import { createHttpServerMethods } from "./backend";
import {
  base64,
  decodeBase64,
  isLoopbackHost,
  isNullBodyStatus,
  nextId,
  resolveOptions,
  validateHost,
  validatePort,
  validateRespondArgs,
} from "./backend-transport";
import { headerPairs, headersFromWire, SET_COOKIE_CARRIER, setCookieHeader } from "./headers";
import type { HttpRequestPayload } from "./types";

describe("isLoopbackHost", () => {
  test("accepts localhost and literal loopback IPs", () => {
    for (const host of [
      "localhost",
      "LOCALHOST",
      "127.0.0.1",
      "127.1.2.3",
      "::1",
      "[::1]",
      "0:0:0:0:0:0:0:1",
      "::ffff:127.0.0.1",
    ]) {
      expect(isLoopbackHost(host)).toBe(true);
    }
  });

  test("rejects DNS names that merely start with 127. and non-loopback addresses", () => {
    for (const host of [
      "127.0.0.1.nip.io",
      "127.evil.example.com",
      "127.x.192.168.1.5.nip.io",
      "localhost.evil.com",
      "127.1",
      "0.0.0.0",
      "::",
      "192.168.1.5",
      "::ffff:192.168.1.5",
      "example.com",
    ]) {
      expect(isLoopbackHost(host)).toBe(false);
    }
  });
});

describe("response headers", () => {
  test("headerPairs keeps every Set-Cookie", () => {
    const headers = new Headers({ "x-a": "1" });
    headers.append("set-cookie", "a=1; Path=/");
    headers.append("set-cookie", "b=2; Path=/");
    const pairs = headerPairs(headers);
    expect(pairs.filter(([name]) => name === "set-cookie")).toEqual([
      ["set-cookie", "a=1; Path=/"],
      ["set-cookie", "b=2; Path=/"],
    ]);
    expect(pairs).toContainEqual(["x-a", "1"]);
    expect(headersFromWire(pairs).getSetCookie()).toEqual(["a=1; Path=/", "b=2; Path=/"]);
  });

  test("setCookieHeader carrier becomes real Set-Cookie pairs", () => {
    const headers = new Headers([
      ["x-a", "1"],
      setCookieHeader(["a=1; Expires=Wed, 21 Oct 2026 07:28:00 GMT", "b=2"]),
    ]);
    const pairs = headerPairs(headers);
    expect(pairs.some(([name]) => name === SET_COOKIE_CARRIER)).toBe(false);
    expect(pairs.filter(([name]) => name === "set-cookie")).toEqual([
      ["set-cookie", "a=1; Expires=Wed, 21 Oct 2026 07:28:00 GMT"],
      ["set-cookie", "b=2"],
    ]);
    expect(pairs).toContainEqual(["x-a", "1"]);
  });

  test("a malformed carrier is ignored instead of throwing", () => {
    const headers = new Headers([[SET_COOKIE_CARRIER, "not json"]]);
    expect(headerPairs(headers)).toEqual([]);
  });

  test.each(["a\r\nb", "a\nb", "a\rb", "a\0b"])(
    "a carrier cookie with CR/LF/NUL throws in headerPairs (%j)",
    (value) => {
      const headers = new Headers([setCookieHeader([`q=${value}`])]);
      expect(() => headerPairs(headers)).toThrow(TypeError);
    },
  );

  test("headerPairs works when getSetCookie is unavailable", () => {
    const headers = new Headers([["x-a", "1"]]);
    headers.append("set-cookie", "a=1");
    Object.defineProperty(headers, "getSetCookie", { value: undefined });
    const pairs = headerPairs(headers);
    expect(pairs).toContainEqual(["x-a", "1"]);
    expect(pairs).toContainEqual(["set-cookie", "a=1"]);
  });

  test("headersFromWire still accepts the record form and null", () => {
    expect(headersFromWire({ "x-test": "yes" }).get("x-test")).toBe("yes");
    expect([...headersFromWire(null)]).toEqual([]);
  });
});

describe("option and argument validation", () => {
  test("resolveOptions applies defaults, clamps and rejects bad integers", () => {
    const defaults = resolveOptions({});
    expect(defaults.streaming).toBe(false);
    expect(defaults.websocket).toBe(false);
    expect(defaults.maxPendingRequests).toBeGreaterThan(0);
    expect(resolveOptions({ streaming: true, websocket: true })).toMatchObject({
      streaming: true,
      websocket: true,
    });
    expect(() => resolveOptions({ maxPendingRequests: 0 })).toThrow(TypeError);
    expect(() => resolveOptions({ bodyCapBytes: 1.5 })).toThrow(TypeError);
    expect(() => resolveOptions({ responseTimeoutSecs: Number.NaN })).toThrow(TypeError);
    expect(resolveOptions({ maxPendingRequests: Number.MAX_SAFE_INTEGER }).maxPendingRequests).toBe(
      resolveOptions({ maxPendingRequests: Number.MAX_SAFE_INTEGER - 1 }).maxPendingRequests,
    );
  });

  test("validatePort / validateHost", () => {
    expect(() => validatePort(0)).not.toThrow();
    expect(() => validatePort(65_535)).not.toThrow();
    expect(() => validatePort(65_536)).toThrow(TypeError);
    expect(() => validatePort(-1)).toThrow(TypeError);
    expect(() => validatePort(1.2)).toThrow(TypeError);
    expect(() => validateHost("127.0.0.1")).not.toThrow();
    expect(() => validateHost("")).toThrow(TypeError);
    expect(() => validateHost("a\0b")).toThrow(TypeError);
    expect(() => validateHost("a".repeat(254))).toThrow(TypeError);
  });

  test("validateRespondArgs enforces status range and canonical base64", () => {
    expect(validateRespondArgs({ serverId: 1, reqId: 1 })).toEqual({
      status: 200,
      body: new Uint8Array(),
    });
    expect(validateRespondArgs({ serverId: 1, reqId: 1, status: 201, body: "aGk=" }).body).toEqual(
      new Uint8Array([104, 105]),
    );
    expect(() => validateRespondArgs({ serverId: 1, reqId: 1, status: 199 })).toThrow(TypeError);
    expect(() => validateRespondArgs({ serverId: 1, reqId: 1, status: 600 })).toThrow(TypeError);
    expect(() => validateRespondArgs({ serverId: 1, reqId: 1, body: "not base64!" })).toThrow(
      TypeError,
    );
  });

  test("base64 helpers round-trip and null-body statuses", () => {
    const bytes = new Uint8Array([0, 1, 2, 250, 255]);
    expect(decodeBase64(base64(bytes))).toEqual(bytes);
    expect(isNullBodyStatus(204)).toBe(true);
    expect(isNullBodyStatus(304)).toBe(true);
    expect(isNullBodyStatus(200)).toBe(false);
  });

  test("nextId skips used ids and wraps", () => {
    expect(nextId(0, () => false)).toBe(1);
    expect(nextId(1, (id) => id === 2 || id === 3)).toBe(4);
    expect(nextId(Number.MAX_SAFE_INTEGER, () => false)).toBe(1);
  });
});

function backend() {
  const requests: HttpRequestPayload[] = [];
  const binds: Array<[string, number]> = [];
  const methods = createHttpServerMethods({
    emit: (event, payload) => {
      if (event === "http:request") requests.push(payload as HttpRequestPayload);
    },
    checkBind: async (host, port) => {
      binds.push([host, port]);
    },
  });
  return { methods, requests, binds };
}

describe("backend lifecycle", () => {
  test("reserve validates and consults the bind policy before listening", async () => {
    const { methods, binds } = backend();
    await expect(methods.reserve({ port: 70_000 })).rejects.toThrow("port");
    await expect(methods.reserve({ host: "" })).rejects.toThrow("host");
    expect(binds).toEqual([]);
    const { id } = await methods.reserve({});
    expect(binds).toEqual([["127.0.0.1", 0]]);
    expect(methods.list()).toEqual([]);
    await expect(methods.activate({ serverId: id + 100 })).rejects.toThrow("not found");
  });

  test("a denied bind policy fails reserve", async () => {
    const methods = createHttpServerMethods({
      emit: () => undefined,
      checkBind: async () => {
        throw new Error("denied");
      },
    });
    await expect(methods.reserve({})).rejects.toThrow("denied");
  });
});

describe("backend request flow", () => {
  test("serves a request, answers via respond() and stops cleanly", async () => {
    const { methods, requests } = backend();
    const { id } = await methods.reserve({});
    const served = await methods.activate({ serverId: id });
    expect(served.host).toBe("127.0.0.1");
    expect(methods.list()).toEqual([id]);
    // Activating twice returns the same listener.
    expect((await methods.activate({ serverId: id })).port).toBe(served.port);

    const pending = fetch(`http://127.0.0.1:${served.port}/hello?x=1`, { method: "GET" });
    const deadline = Date.now() + 2000;
    while (requests.length === 0 && Date.now() < deadline) await Bun.sleep(5);
    expect(requests).toHaveLength(1);
    const [request] = requests;
    expect(request?.method).toBe("GET");
    expect(request?.path).toBe("/hello?x=1");

    methods.respond({
      serverId: id,
      reqId: request?.reqId ?? -1,
      status: 201,
      headers: { "x-test": "yes" },
      body: base64(new TextEncoder().encode("hi")),
    });
    const response = await pending;
    expect(response.status).toBe(201);
    expect(response.headers.get("x-test")).toBe("yes");
    expect(await response.text()).toBe("hi");

    // Responding to an unknown request is a no-op.
    expect(() => methods.respond({ serverId: id, reqId: 9999 })).not.toThrow();

    expect(await methods.stop({ serverId: id })).toEqual({ stopped: true });
    expect(methods.list()).toEqual([]);
    expect(await methods.stop({ serverId: id })).toEqual({ stopped: false });
  });
});

describe("backend set-cookie", () => {
  test("respond() with header pairs delivers every Set-Cookie to the client", async () => {
    const { methods, requests } = backend();
    const { id } = await methods.reserve({});
    const served = await methods.activate({ serverId: id });
    try {
      const pending = fetch(`http://127.0.0.1:${served.port}/`);
      const deadline = Date.now() + 2000;
      while (requests.length === 0 && Date.now() < deadline) await Bun.sleep(5);
      methods.respond({
        serverId: id,
        reqId: requests[0]?.reqId ?? -1,
        headers: [
          ["set-cookie", "a=1"],
          ["set-cookie", "b=2"],
          ["x-test", "yes"],
        ],
      });
      const response = await pending;
      expect(response.headers.getSetCookie()).toEqual(["a=1", "b=2"]);
      expect(response.headers.get("x-test")).toBe("yes");
    } finally {
      await methods.stop({ serverId: id });
    }
  });
});
