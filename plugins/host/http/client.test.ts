import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { http } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-http client", () => {
  test("getJson maps to a json request", async () => {
    await http.getJson("https://x.test");
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "http",
        method: "request",
        args: { url: "https://x.test", method: "GET", responseType: "json" },
      },
    ]);
  });

  test("post maps to a POST request", async () => {
    await http.post("https://x.test", { body: { a: 1 } });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "http",
        method: "request",
        args: {
          url: "https://x.test",
          method: "POST",
          responseType: "text",
          body: { a: 1 },
        },
      },
    ]);
  });
});

describe("@tynd/plugin-http transport options", () => {
  test("transport options are forwarded, signal is not", async () => {
    const controller = new AbortController();
    await http.get("https://x.test", {
      redirect: "manual",
      maxRedirects: 3,
      connectTimeoutMs: 500,
      proxy: "socks5h://127.0.0.1:9050",
      cookies: "session",
      signal: controller.signal,
    });
    const call = bridge.calls[0] as { args: Record<string, unknown> };
    expect(call.args).toMatchObject({
      url: "https://x.test",
      redirect: "manual",
      maxRedirects: 3,
      connectTimeoutMs: 500,
      proxy: "socks5h://127.0.0.1:9050",
      cookies: "session",
    });
    expect(typeof call.args.abortId).toBe("string");
    expect("signal" in call.args).toBe(false);
  });

  test("an already aborted signal rejects without calling the host", async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(http.get("https://x.test", { signal: controller.signal })).rejects.toBeDefined();
    expect(bridge.calls).toEqual([]);
  });

  test("aborting mid-flight rejects and calls http.abort with the same id", async () => {
    bridge.setOs("http", "request", () => new Promise(() => undefined));
    const controller = new AbortController();
    const pending = http.get("https://x.test", { signal: controller.signal });
    controller.abort(new Error("stop"));
    await expect(pending).rejects.toThrow("stop");
    const [request, abort] = bridge.calls as { method: string; args: Record<string, unknown> }[];
    expect(abort?.method).toBe("abort");
    expect(abort?.args.id).toBe(request?.args.abortId);
  });

  test("clearCookies", async () => {
    await http.clearCookies("session");
    await http.clearCookies();
    expect(bridge.calls.map((c) => (c as { args: unknown }).args)).toEqual([
      { jar: "session" },
      {},
    ]);
  });
});

describe("@tynd/plugin-http stream", () => {
  test("resolves on the head event and yields chunks until end", async () => {
    let streamId = "";
    bridge.setOs("http", "stream", (args) => {
      streamId = (args as { streamId: string }).streamId;
      queueMicrotask(() => {
        bridge.emitOs("http:stream", { id: "other", kind: "head", status: 500 });
        bridge.emitOs("http:stream", {
          id: streamId,
          kind: "head",
          status: 200,
          statusText: "OK",
          headers: { "content-type": "text/plain" },
          url: "https://x.test/final",
          redirected: true,
        });
        bridge.emitOs("http:stream", { id: streamId, kind: "chunk", data: "aGk=" });
        bridge.emitOs("http:stream", { id: streamId, kind: "chunk", data: "IQ==" });
        bridge.emitOs("http:stream", { id: streamId, kind: "end", bytes: 3 });
      });
      return new Promise(() => undefined);
    });
    const res = await http.stream("https://x.test");
    expect(res.status).toBe(200);
    expect(res.url).toBe("https://x.test/final");
    expect(res.redirected).toBe(true);
    const text = await new Response(res.body).text();
    expect(text).toBe("hi!");
  });

  test("a host error before the head rejects", async () => {
    bridge.setOs("http", "stream", () => {
      throw new Error("http.stream: connection refused");
    });
    await expect(http.stream("https://x.test")).rejects.toThrow("connection refused");
  });
});
