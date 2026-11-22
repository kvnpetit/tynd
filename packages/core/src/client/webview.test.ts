import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { tyndWebview } from "./webview";

type Call = { api: string; method: string; args: unknown };
type Listener = (data: unknown) => void;

const calls: Call[] = [];
const listeners = new Map<string, Set<Listener>>();
let nextResult: unknown = null;

function emit(name: string, data: unknown) {
  for (const listener of listeners.get(name) ?? []) listener(data);
}

beforeEach(() => {
  calls.length = 0;
  listeners.clear();
  nextResult = null;
  (globalThis as { window?: unknown }).window = {
    __TYND_WINDOW_LABEL__: "main",
    __TYND_WEBVIEW_LABEL__: "pane",
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        calls.push({ api, method, args });
        return Promise.resolve(nextResult);
      },
      os_on(name: string, handler: Listener) {
        const set = listeners.get(name) ?? new Set<Listener>();
        set.add(handler);
        listeners.set(name, set);
        return () => {
          set.delete(handler);
        };
      },
    },
  };
});

afterEach(() => {
  (globalThis as { window?: unknown }).window = undefined;
});

describe("tyndWebview.cookies", () => {
  test("get targets the caller's webview by default", async () => {
    nextResult = [{ name: "a", value: "b" }];
    const cookies = await tyndWebview.cookies.get();
    expect(cookies).toEqual([{ name: "a", value: "b" } as never]);
    expect(calls).toEqual([
      { api: "window", method: "getWebviewCookies", args: { label: "pane" } },
    ]);
  });

  test("get forwards an explicit label and url", async () => {
    await tyndWebview.cookies.get({ label: "other", url: "https://example.com/" });
    expect(calls[0]?.args).toEqual({ label: "other", url: "https://example.com/" });
  });

  test("set sends the cookie object next to the target", async () => {
    await tyndWebview.cookies.set(
      { name: "sid", value: "1", httpOnly: true },
      { url: "https://example.com/" },
    );
    expect(calls).toEqual([
      {
        api: "window",
        method: "setWebviewCookie",
        args: {
          label: "pane",
          url: "https://example.com/",
          cookie: { name: "sid", value: "1", httpOnly: true },
        },
      },
    ]);
  });

  test("delete flattens the cookie key into the arguments", async () => {
    await tyndWebview.cookies.delete({ name: "sid", domain: "example.com", path: "/" });
    expect(calls).toEqual([
      {
        api: "window",
        method: "deleteWebviewCookie",
        args: { label: "pane", name: "sid", domain: "example.com", path: "/" },
      },
    ]);
  });

  test("clear resolves to the removed count", async () => {
    nextResult = 3;
    expect(await tyndWebview.cookies.clear({ label: "x" })).toBe(3);
    expect(calls[0]).toEqual({
      api: "window",
      method: "clearWebviewCookies",
      args: { label: "x" },
    });
  });
});

describe("tyndWebview.onDownload", () => {
  test("tags started and completed events and unsubscribes both", () => {
    const seen: unknown[] = [];
    const off = tyndWebview.onDownload((event) => seen.push(event));
    emit("webview:download-started", {
      label: "main",
      id: 7,
      url: "https://example.com/f.zip",
      path: "C:/Users/u/Downloads/f.zip",
    });
    emit("webview:download-completed", {
      label: "main",
      id: 7,
      url: "https://example.com/f.zip",
      path: "C:/Users/u/Downloads/f.zip",
      success: true,
    });
    expect(seen).toEqual([
      {
        type: "started",
        label: "main",
        id: 7,
        url: "https://example.com/f.zip",
        path: "C:/Users/u/Downloads/f.zip",
      },
      {
        type: "completed",
        label: "main",
        id: 7,
        url: "https://example.com/f.zip",
        path: "C:/Users/u/Downloads/f.zip",
        success: true,
      },
    ]);
    off();
    emit("webview:download-started", { label: "main", id: 8, url: "u", path: "p" });
    expect(seen).toHaveLength(2);
  });
});

describe("tyndWebview.onNewWindow", () => {
  test("delivers the host decision", () => {
    const seen: unknown[] = [];
    const off = tyndWebview.onNewWindow((event) => seen.push(event));
    emit("webview:new-window", { label: "main", url: "https://a.b/", action: "external" });
    expect(seen).toEqual([{ label: "main", url: "https://a.b/", action: "external" }]);
    off();
    emit("webview:new-window", { label: "main", url: "https://c.d/", action: "deny" });
    expect(seen).toHaveLength(1);
  });
});
