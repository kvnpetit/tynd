import { describe, expect, test } from "bun:test";
import { createOAuthMethods, validateOAuthPath, validateSuccessHtml } from "./backend";
import { createOAuthFlow, type FlowDependencies } from "./flow";
import type { OAuthCallbackEvent, OAuthErrorEvent } from "./types";

function harness() {
  const callbackHandlers = new Set<(payload: Partial<OAuthCallbackEvent>) => void>();
  const errorHandlers = new Set<(payload: Partial<OAuthErrorEvent>) => void>();
  const cancelled: number[] = [];
  const dependencies: FlowDependencies = {
    async cancel(id) {
      cancelled.push(id);
    },
    onCallback(handler) {
      callbackHandlers.add(handler);
      return () => callbackHandlers.delete(handler);
    },
    onError(handler) {
      errorHandlers.add(handler);
      return () => errorHandlers.delete(handler);
    },
  };
  return {
    dependencies,
    cancelled,
    callbackHandlers,
    errorHandlers,
    callback: (payload: Partial<OAuthCallbackEvent>) => {
      for (const handler of [...callbackHandlers]) handler(payload);
    },
    error: (payload: Partial<OAuthErrorEvent>) => {
      for (const handler of [...errorHandlers]) handler(payload);
    },
  };
}

const handle = { id: 7, port: 4242, redirectUri: "http://127.0.0.1:4242/" };

describe("createOAuthFlow", () => {
  test("resolves with the params of this flow's callback and ignores other ids", async () => {
    const h = harness();
    const flow = createOAuthFlow(handle, h.dependencies);
    const waiting = flow.wait();
    h.callback({ id: 99, params: { code: "other" } });
    h.callback({ id: 7, params: { code: "abc", state: "s" } });
    expect(await waiting).toEqual({ params: { code: "abc", state: "s" } });
    // Listeners are released once settled, and late waits still resolve.
    expect(h.callbackHandlers.size).toBe(0);
    expect(h.errorHandlers.size).toBe(0);
    expect(await flow.wait()).toEqual({ params: { code: "abc", state: "s" } });
  });

  test("rejects on a listener error event", async () => {
    const h = harness();
    const flow = createOAuthFlow(handle, h.dependencies);
    const waiting = flow.wait();
    h.error({ id: 7, error: "state mismatch" });
    await expect(waiting).rejects.toThrow("oauth: state mismatch");
    await expect(flow.wait()).rejects.toThrow("state mismatch");
  });

  test("cancel notifies the backend once and rejects pending waiters", async () => {
    const h = harness();
    const flow = createOAuthFlow(handle, h.dependencies);
    const waiting = flow.wait();
    await Promise.all([flow.cancel(), flow.cancel()]);
    expect(h.cancelled).toEqual([7]);
    await expect(waiting).rejects.toThrow("flow cancelled");
    // Cancelling a settled flow is a no-op.
    await flow.cancel();
    expect(h.cancelled).toEqual([7]);
  });

  test("wait times out, rejects, and cancels the backend listener", async () => {
    const h = harness();
    const flow = createOAuthFlow(handle, h.dependencies);
    await expect(flow.wait({ timeoutMs: 20 })).rejects.toThrow("timed out after 20ms");
    expect(h.cancelled).toEqual([7]);
  });
});

describe("validators", () => {
  test("validateOAuthPath", () => {
    expect(() => validateOAuthPath("/callback")).not.toThrow();
    expect(() => validateOAuthPath("callback")).toThrow(TypeError);
    expect(() => validateOAuthPath("//evil")).toThrow(TypeError);
    expect(() => validateOAuthPath("/a?b=1")).toThrow(TypeError);
    expect(() => validateOAuthPath("/a#b")).toThrow(TypeError);
    expect(() => validateOAuthPath("/café")).toThrow(TypeError);
    expect(() => validateOAuthPath(`/${"a".repeat(300)}`)).toThrow(TypeError);
  });

  test("validateSuccessHtml", () => {
    expect(() => validateSuccessHtml("<p>ok</p>")).not.toThrow();
    expect(() => validateSuccessHtml("<p>café</p>")).toThrow(TypeError);
    expect(() => validateSuccessHtml("a\0b")).toThrow(TypeError);
    expect(() => validateSuccessHtml("a".repeat(5000))).toThrow(TypeError);
  });
});

function backend() {
  const events: Array<{ event: string; payload: unknown }> = [];
  const binds: number[] = [];
  const methods = createOAuthMethods({
    emit: (event, payload) => {
      events.push({ event, payload });
    },
    checkBind: async (port) => {
      binds.push(port);
    },
  });
  return { methods, events, binds };
}

describe("backend validation", () => {
  test("rejects invalid arguments before binding", async () => {
    const { methods, binds } = backend();
    await expect(methods.start({ port: -1 })).rejects.toThrow("port");
    await expect(methods.start({ port: 70_000 })).rejects.toThrow("port");
    await expect(methods.start({ path: "nope" })).rejects.toThrow("path");
    await expect(methods.start({ expectedState: "bad\nstate" })).rejects.toThrow("expectedState");
    expect(binds).toEqual([]);
    expect(methods.list()).toEqual([]);
  });

  test("a denied loopback bind policy fails the start", async () => {
    const methods = createOAuthMethods({
      emit: () => undefined,
      checkBind: async () => {
        throw new Error("bind denied");
      },
    });
    await expect(methods.start({})).rejects.toThrow("bind denied");
    expect(methods.list()).toEqual([]);
  });
});

describe("backend loopback listener", () => {
  test("serves the callback once, emits sanitized params and tears down", async () => {
    const { methods, events } = backend();
    const started = await methods.start({ path: "/cb", expectedState: "xyz" });
    expect(started.redirectUri).toBe(`http://127.0.0.1:${started.port}/cb`);
    expect(methods.list()).toEqual([started.id]);

    const notFound = await fetch(`http://127.0.0.1:${started.port}/other`);
    expect(notFound.status).toBe(404);

    const ok = await fetch(`${started.redirectUri}?code=abc&state=xyz`);
    expect(ok.status).toBe(200);
    expect(ok.headers.get("content-type")).toContain("text/html");
    expect(ok.headers.get("content-security-policy")).toContain("script-src 'none'");
    await ok.text();

    expect(events).toEqual([
      {
        event: "oauth:callback",
        payload: { id: started.id, params: { code: "abc", state: "xyz" } },
      },
    ]);
    await Bun.sleep(20);
    expect(methods.list()).toEqual([]);
  });

  test("a state mismatch emits an error, answers 400 and closes the listener", async () => {
    const { methods, events } = backend();
    const started = await methods.start({ expectedState: "good" });
    const bad = await fetch(`${started.redirectUri}?code=abc&state=evil`);
    expect(bad.status).toBe(400);
    await bad.text();
    expect(events).toEqual([
      { event: "oauth:error", payload: { id: started.id, error: "state mismatch" } },
    ]);
    await Bun.sleep(20);
    expect(methods.list()).toEqual([]);
  });

  test("cancel stops the listener and is idempotent", async () => {
    const { methods } = backend();
    const started = await methods.start({});
    await methods.cancel({ id: started.id });
    await methods.cancel({ id: started.id });
    expect(methods.list()).toEqual([]);
    await expect(fetch(started.redirectUri)).rejects.toThrow();
  });
});

describe("backend flow limit", () => {
  test("concurrent starts cannot exceed the flow cap across the bind check", async () => {
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const methods = createOAuthMethods({
      emit: () => undefined,
      checkBind: () => gate,
    });
    const attempts = Array.from({ length: 20 }, () => methods.start({}));
    release();
    const results = await Promise.allSettled(attempts);
    try {
      expect(results.filter((r) => r.status === "fulfilled")).toHaveLength(16);
      expect(methods.list()).toHaveLength(16);
      const rejected = results.filter((r) => r.status === "rejected");
      expect(rejected).toHaveLength(4);
      for (const r of rejected)
        expect(String((r as PromiseRejectedResult).reason)).toContain("flow limit");
    } finally {
      for (const id of methods.list()) await methods.cancel({ id });
    }
  });
});
