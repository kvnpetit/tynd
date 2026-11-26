import { afterEach, describe, expect, test } from "bun:test";
import { TyndIpcError } from "@tynd/core/client";
import { createHandleEvents, defineTypedPlugin, onHostEvent } from "./index";

const g = globalThis as Record<string, unknown>;
afterEach(() => {
  Reflect.deleteProperty(g, "window");
  Reflect.deleteProperty(g, "__tynd__");
});

describe("defineTypedPlugin native route", () => {
  test('calls go through __tynd__.os_call("plugin", "<name>:<method>")', async () => {
    const calls: unknown[][] = [];
    g.window = {
      __tynd__: {
        os_call: (api: string, method: string, args: unknown) => {
          calls.push([api, method, args]);
          return Promise.resolve("pong");
        },
      },
    };
    type Catalog = { ping: { args: { n: number }; result: string } };
    const plugin = defineTypedPlugin<Catalog>("foo");
    await expect(plugin.ping({ n: 1 })).resolves.toBe("pong");
    expect(calls).toEqual([["plugin", "foo:ping", { n: 1 }]]);
  });

  test("a dead backend rejects with a typed TyndIpcError (core bridge decoding)", async () => {
    g.window = {
      __tynd__: {
        os_call: () =>
          Promise.reject(Object.assign(new Error("backend exited"), { code: "BACKEND_DEAD" })),
      },
    };
    type Catalog = { ping: { args: null; result: string } };
    const plugin = defineTypedPlugin<Catalog>("foo");
    const err = await plugin.ping().then(
      () => null,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(TyndIpcError);
    expect((err as TyndIpcError).code).toBe("BACKEND_DEAD");
  });

  test("rejects outside a Tynd webview instead of reaching the Bun registry", async () => {
    type Catalog = { ping: { args: null; result: string } };
    const plugin = defineTypedPlugin<Catalog>("foo");
    await expect(plugin.ping()).rejects.toThrow(/__tynd__\.os_call missing/);
  });
});

type Listener = (payload: unknown) => void;

/** Fake shim: native plugin events go to `os_on`; `on` is the backend channel and must stay unused. */
function fakeShim() {
  const osListeners = new Map<string, Listener[]>();
  const backendOn: string[] = [];
  g.window = {
    __tynd__: {
      on: (name: string) => {
        backendOn.push(name);
        return () => undefined;
      },
      os_on: (name: string, handler: Listener) => {
        const list = osListeners.get(name) ?? [];
        list.push(handler);
        osListeners.set(name, list);
        return () => {
          const idx = list.indexOf(handler);
          if (idx >= 0) list.splice(idx, 1);
        };
      },
    },
  };
  g.__tynd__ = (g.window as { __tynd__: unknown }).__tynd__;
  // What `window.__tynd_os_event__` does when the host emits a plugin event.
  const emit = (name: string, data: unknown) => {
    for (const handler of [...(osListeners.get(name) ?? [])]) handler(data);
  };
  return { emit, backendOn, osListeners };
}

describe("host event subscription", () => {
  test("onHostEvent receives native plugin events from the OS event bus", () => {
    const shim = fakeShim();
    const seen: unknown[] = [];
    const off = onHostEvent("terminal:data", (p) => seen.push(p));
    shim.emit("terminal:data", { id: 1, data: "hi" });
    off();
    shim.emit("terminal:data", { id: 1, data: "late" });
    expect(seen).toEqual([{ id: 1, data: "hi" }]);
    expect(shim.backendOn).toEqual([]);
  });

  test("onHostEvent is a no-op outside a Tynd webview", () => {
    expect(() => onHostEvent("terminal:data", () => undefined)()).not.toThrow();
  });

  test("createHandleEvents filters OS-bus events by handle id and disposes", async () => {
    const shim = fakeShim();
    const events = createHandleEvents(7, "test");
    const seen: unknown[] = [];
    events.on<{ id: number; v: string }>("probe:data", "id", (p) => seen.push(p.v));
    events.disposeOn("probe:exit", "id");
    shim.emit("probe:data", { id: 8, v: "other" });
    shim.emit("probe:data", { id: 7, v: "mine" });
    shim.emit("probe:exit", { id: 7 });
    await Promise.resolve();
    shim.emit("probe:data", { id: 7, v: "after" });
    expect(seen).toEqual(["mine"]);
    expect(shim.osListeners.get("probe:data")).toEqual([]);
    expect(shim.backendOn).toEqual([]);
  });
});
