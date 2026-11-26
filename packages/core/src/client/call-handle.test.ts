import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { TyndIpcError } from "../errors";
import { installTyndMock, type TyndMock } from "../testing";
import { osDeadline } from "./_internal";
import { createBackend } from "./backend";
import { createPluginClient } from "./plugin";

const g = globalThis as Record<string, unknown>;

interface Posted {
  type: string;
  id: string;
  fn?: string;
}

/**
 * Evaluate the real webview shim (`JS_SHIM` in crates/host/src/webview/shim.rs) against a fake
 * `window`, so call/cancel/os_call semantics are tested on the shipped code.
 */
/** Token literal of the unfilled `JS_SHIM` template (`ipc_shim` substitutes the real one). */
const TOKEN = "__TYND_IPC_TOKEN__";

function shimSource(): string {
  const path = fileURLToPath(
    new URL("../../../../crates/host/src/webview/shim.rs", import.meta.url),
  );
  const source = readFileSync(path, "utf8");
  const match = /pub\(crate\) const JS_SHIM: &str = r#"([\s\S]*?)"#;/.exec(source);
  if (!match?.[1]) throw new Error("JS_SHIM not found in crates/host/src/webview/shim.rs");
  return match[1];
}

/**
 * Evaluate the shim in a fake frame. `frame` makes it a subframe at that location (the top
 * frame otherwise); `raw` collects the messages exactly as posted, token prefix included.
 */
function evalShim(
  frame?: { protocol: string; origin: string },
  onPost?: (m: string) => void,
  topLocation?: { protocol: string; origin: string; href: string },
) {
  const raw: string[] = [];
  const postMessage = (m: string) => {
    raw.push(m);
    onPost?.(m);
  };
  // biome-ignore lint/suspicious/noExplicitAny: the shim writes untyped hooks on the fake window
  const win: any = { ipc: { postMessage } };
  if (frame) {
    win.top = {};
    win.location = frame;
  } else {
    win.top = win;
    if (topLocation) win.location = topLocation;
  }
  new Function("window", shimSource())(win);
  return { win, raw };
}

function loadShim() {
  const posted: Posted[] = [];
  const { win } = evalShim(undefined, (m) => {
    if (!m.startsWith(TOKEN)) throw new Error(`message without the IPC token: ${m}`);
    posted.push(JSON.parse(m.slice(TOKEN.length)) as Posted);
  });
  g.window = win;
  return { win, posted };
}

afterEach(() => {
  Reflect.deleteProperty(g, "window");
});

/** Non-stream calls are typed `Promise<T>`, but the runtime handle still carries `cancel()`. */
type Cancellable<T> = Promise<T> & { cancel(): Promise<unknown> };

const tick = (ms = 5) => new Promise((resolve) => setTimeout(resolve, ms));

async function caught(p: PromiseLike<unknown>): Promise<unknown> {
  try {
    await p;
  } catch (e) {
    return e;
  }
  return;
}

type Api = { tail(): AsyncGenerator<number, string>; save(x: number): Promise<number> };

describe("real shim: cancel", () => {
  test("cancelling a call rejects it with a CANCELLED TyndIpcError and frees the entry", async () => {
    const { win, posted } = loadShim();
    const api = createBackend<Api>();
    const handle = api.tail();
    const id = (posted[0] as Posted).id;
    await handle.cancel();
    expect(posted.some((m) => m.type === "cancel" && m.id === id)).toBe(true);

    const error = await caught(handle);
    expect(error).toBeInstanceOf(TyndIpcError);
    expect((error as TyndIpcError).code).toBe("CANCELLED");
    expect((error as TyndIpcError).fn).toBe("tail");

    // The entry is gone: late frames from the host are ignored.
    win.__tynd_yield__(id, "1");
    win.__tynd_resolve__(id, true, '"late"');
    expect((await caught(handle)) as TyndIpcError).toMatchObject({ code: "CANCELLED" });
  });

  test("cancel rejects a for-await loop parked in next()", async () => {
    const { posted } = loadShim();
    const api = createBackend<Api>();
    const handle = api.tail();
    const loop = (async () => {
      for await (const _ of handle) {
        /* never yields */
      }
    })();
    await tick();
    expect(posted.length).toBe(1);
    await handle.cancel();
    const error = await caught(loop);
    expect(error).toBeInstanceOf(TyndIpcError);
    expect((error as TyndIpcError).code).toBe("CANCELLED");
  });
});

describe("real shim: cancel via for-await and settled calls", () => {
  test("breaking out of for-await cancels without an unhandled rejection", async () => {
    const { win, posted } = loadShim();
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      const api = createBackend<Api>();
      const handle = api.tail();
      const id = (posted[0] as Posted).id;
      win.__tynd_yield__(id, "1");
      const seen: number[] = [];
      for await (const n of handle) {
        seen.push(n);
        break;
      }
      expect(seen).toEqual([1]);
      expect(posted.some((m) => m.type === "cancel" && m.id === id)).toBe(true);
      await tick(20);
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });

  test("a stream error caught around for-await raises no unhandled rejection", async () => {
    const { win, posted } = loadShim();
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      const handle = createBackend<Api>().tail();
      const id = (posted[0] as Posted).id;
      win.__tynd_yield__(id, "1");
      let message = "";
      const loop = (async () => {
        try {
          for await (const _ of handle) {
            win.__tynd_resolve__(id, false, "boom");
          }
        } catch (e) {
          message = (e as Error).message;
        }
      })();
      await loop;
      expect(message).toContain("boom");
      await tick(20);
      expect(unhandled).toEqual([]);
      // The final promise still reports the failure to whoever awaits it.
      expect(await caught(handle)).toBeInstanceOf(Error);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });

  test("cancel after the call settled is a no-op", async () => {
    const { win, posted } = loadShim();
    const handle = createBackend<Api>().save(1) as Cancellable<number>;
    win.__tynd_resolve__((posted[0] as Posted).id, true, "7");
    expect(await handle).toBe(7);
    await handle.cancel();
    expect(posted.some((m) => m.type === "cancel")).toBe(false);
  });
});

describe("real shim: BACKEND_DEAD", () => {
  test("a dead backend rejects pending calls with a typed BACKEND_DEAD error", async () => {
    const { win } = loadShim();
    const handle = createBackend<{ save(): Promise<void> }>().save();
    win.__tynd_backend_dead__("bun exited");
    const error = await caught(handle);
    expect(error).toBeInstanceOf(TyndIpcError);
    expect((error as TyndIpcError).code).toBe("BACKEND_DEAD");
  });
});

describe("real shim: os_call deadline", () => {
  test("no timer is armed by default (dialogs, downloads and process runs may take minutes)", async () => {
    const { win, posted } = loadShim();
    const timers: unknown[] = [];
    const realSetTimeout = globalThis.setTimeout;
    globalThis.setTimeout = ((...args: unknown[]) => {
      timers.push(args);
      return (realSetTimeout as (...a: unknown[]) => unknown)(...args);
    }) as typeof setTimeout;
    try {
      const p = win.__tynd__.os_call("dialog", "openFile", {});
      expect(timers).toEqual([]);
      await tick(20);
      const id = (posted[0] as Posted).id;
      win.__tynd_os_result__(id, true, "chosen.txt");
      expect(await p).toBe("chosen.txt");
    } finally {
      globalThis.setTimeout = realSetTimeout;
    }
  });

  test("a caller-provided timeoutMs rejects and ignores the late result", async () => {
    const { win, posted } = loadShim();
    const p = win.__tynd__.os_call("http", "download", {}, { timeoutMs: 15 });
    await expect(p).rejects.toThrow(/timed out after 15ms/);
    win.__tynd_os_result__((posted[0] as Posted).id, true, "late");
  });

  test("osDeadline adds slack to a request timeout and is absent without one", () => {
    expect(osDeadline(undefined)).toBeUndefined();
    expect(osDeadline(0)).toBeUndefined();
    expect(osDeadline(600_000)?.timeoutMs).toBeGreaterThan(600_000);
  });
});

describe("plugin client call handles", () => {
  let mock: TyndMock | undefined;
  afterEach(() => mock?.restore());

  test("createPluginClient supports catch/finally and keeps cancel + iteration", async () => {
    mock = installTyndMock({
      backend: {
        $plugin$term$run: () => {
          throw new Error("boom");
        },
        $plugin$term$stream: async function* () {
          yield 1;
          yield 2;
          return "end";
        },
      },
    });
    type Shape = {
      __tynd_plugin__: true;
      name: string;
      methods: {
        run(): Promise<void>;
        stream(): AsyncGenerator<number, string>;
      };
    };
    const term = createPluginClient<Shape>("term");
    let finalised = false;
    const message = await term
      .run()
      .catch((e: Error) => e.message)
      .finally(() => {
        finalised = true;
      });
    expect(message).toContain("boom");
    expect(finalised).toBe(true);

    const seen: number[] = [];
    for await (const n of term.stream()) seen.push(n);
    expect(seen).toEqual([1, 2]);
  });
});

describe("mock parity with the real shim", () => {
  let mock: TyndMock | undefined;
  afterEach(() => mock?.restore());

  test("cancelling a stream rejects the call and a parked next() with CANCELLED", async () => {
    mock = installTyndMock({
      backend: {
        tail: async function* () {
          while (true) {
            await tick(1);
            yield 1;
          }
        },
      },
    });
    const api = createBackend<{ tail(): AsyncGenerator<number, string> }>();
    const handle = api.tail();
    const it = handle[Symbol.asyncIterator]();
    const parked = it.next();
    await parked;
    const second = it.next();
    await handle.cancel();
    expect((await caught(handle)) as TyndIpcError).toMatchObject({ code: "CANCELLED" });
    await caught(second);
    expect((await caught(it.next())) as TyndIpcError).toMatchObject({ code: "CANCELLED" });
  });

  test("cancelling a non-stream call rejects it even though the handler still runs", async () => {
    mock = installTyndMock({ backend: { slow: () => tick(30).then(() => 1) } });
    const handle = createBackend<{ slow(): Promise<number> }>().slow() as Cancellable<number>;
    await handle.cancel();
    const error = await caught(handle);
    expect(error).toBeInstanceOf(TyndIpcError);
    expect((error as TyndIpcError).code).toBe("CANCELLED");
  });

  test("os_call has no default deadline but honours timeoutMs", async () => {
    mock = installTyndMock({ os: { "dialog.openFile": () => tick(30).then(() => "a.txt") } });
    expect(await window.__tynd__.os_call("dialog", "openFile", null)).toBe("a.txt");
    await expect(
      window.__tynd__.os_call("dialog", "openFile", null, { timeoutMs: 5 }),
    ).rejects.toThrow(/timed out after 5ms/);
  });
});

describe("real shim: withOptions", () => {
  test("an aborted signal cancels the call and rejects with its reason", async () => {
    const { posted } = loadShim();
    const api = createBackend<Api>();
    const ac = new AbortController();
    const pending = api.withOptions({ signal: ac.signal }).save(1);
    const id = (posted[0] as Posted).id;
    ac.abort();
    const error = await caught(pending);
    expect((error as DOMException).name).toBe("AbortError");
    expect(posted.some((m) => m.type === "cancel" && m.id === id)).toBe(true);
  });

  test("a signal aborted before the call cancels it right away", async () => {
    const { posted } = loadShim();
    const reason = new Error("stop");
    const error = await caught(
      createBackend<Api>()
        .withOptions({ signal: AbortSignal.abort(reason) })
        .save(1),
    );
    expect(error).toBe(reason);
    expect(posted.map((m) => m.type)).toEqual(["call", "cancel"]);
  });

  test("timeout rejects with a TIMEOUT TyndIpcError and cancels the call", async () => {
    const { posted } = loadShim();
    const error = await caught(createBackend<Api>().withOptions({ timeout: 5 }).save(1));
    expect(error).toBeInstanceOf(TyndIpcError);
    expect((error as TyndIpcError).code).toBe("TIMEOUT");
    expect((error as TyndIpcError).fn).toBe("save");
    expect(posted.map((m) => m.type)).toEqual(["call", "cancel"]);
  });

  test("a call that settles in time is untouched and clears its timer", async () => {
    const { win, posted } = loadShim();
    const pending = createBackend<Api>().withOptions({ timeout: 50 }).save(1);
    win.__tynd_resolve__((posted[0] as Posted).id, true, "2");
    expect(await pending).toBe(2);
    await tick(60);
    expect(posted.map((m) => m.type)).toEqual(["call"]);
  });
});

describe("real shim: withOptions on streams and bad input", () => {
  test("abort ends a stream's iteration with the abort reason", async () => {
    const { win, posted } = loadShim();
    const ac = new AbortController();
    const handle = createBackend<Api>().withOptions({ signal: ac.signal }).tail();
    const id = (posted[0] as Posted).id;
    win.__tynd_yield__(id, "1");
    const seen: number[] = [];
    const loop = (async () => {
      for await (const n of handle) {
        seen.push(n);
        ac.abort();
      }
    })();
    const error = await caught(loop);
    expect(seen).toEqual([1]);
    expect((error as DOMException).name).toBe("AbortError");
  });

  test("invalid options fail loudly", () => {
    loadShim();
    const api = createBackend<Api>();
    expect(() => api.withOptions({ timeout: -1 })).toThrow(/timeout/);
    expect(() => api.withOptions({ signal: {} as AbortSignal })).toThrow(/AbortSignal/);
  });
});

describe("real shim: frame trust and IPC token", () => {
  test("every message carries the token and the captured sink is not re-read", async () => {
    const { win, raw } = evalShim();
    g.window = win;
    // A page script replacing the channel later cannot observe the token.
    const sniffed: string[] = [];
    win.ipc.postMessage = (m: string) => sniffed.push(m);
    const os = win.__tynd__.os_call("app", "getInfo", {});
    expect(sniffed).toEqual([]);
    expect(raw.length).toBe(1);
    expect(raw[0]?.startsWith(TOKEN)).toBe(true);
    expect(JSON.parse((raw[0] as string).slice(TOKEN.length))).toMatchObject({
      type: "os_call",
      api: "app",
      method: "getInfo",
    });
    const id = JSON.parse((raw[0] as string).slice(TOKEN.length)).id;
    win.__tynd_os_result__(id, true, 1);
    expect(await os).toBe(1);
  });

  test("a remote or opaque subframe gets no bridge", () => {
    for (const frame of [
      { protocol: "https:", origin: "https://evil.example" },
      { protocol: "about:", origin: "null" },
    ]) {
      const { win, raw } = evalShim(frame);
      expect(win.__tynd__).toBeUndefined();
      expect(win.__tynd_resolve__).toBeUndefined();
      expect(raw).toEqual([]);
    }
  });

  test("an app-content subframe gets the bridge", () => {
    const { win } = evalShim({ protocol: "tynd:", origin: "tynd://localhost" });
    expect(typeof win.__tynd__.os_call).toBe("function");
  });
});

describe("real shim: app and remote tokens", () => {
  test("an app top document posts only after the host hands it the app token", async () => {
    const { win, raw } = evalShim(undefined, undefined, {
      protocol: "tynd:",
      origin: "tynd://localhost",
      href: "tynd://localhost/index.html",
    });
    g.window = win;
    // Only the app-token request goes out, under the remote token.
    expect(raw.length).toBe(1);
    const request = raw[0] as string;
    expect(request.startsWith(`${TOKEN}?tynd-app-token:`)).toBe(true);
    const requestId = request.slice(TOKEN.length + 1);

    const os = win.__tynd__.os_call("app", "getInfo", {});
    expect(raw.length).toBe(1);

    // A page script cannot replace the receiver of the answer.
    const receiver = win.__tynd_os_result__;
    expect(() => {
      win.__tynd_os_result__ = null;
    }).toThrow();
    expect(win.__tynd_os_result__).toBe(receiver);

    win.__tynd_os_result__(requestId, true, "APP_TOKEN");
    expect(raw.length).toBe(2);
    expect(raw[1]?.startsWith("APP_TOKEN")).toBe(true);
    const id = JSON.parse((raw[1] as string).slice("APP_TOKEN".length)).id;
    // A second answer does not replace the token.
    win.__tynd_os_result__(requestId, true, "OTHER");
    win.__tynd__.os_call("app", "getInfo", {});
    expect(raw[2]?.startsWith("APP_TOKEN")).toBe(true);

    win.__tynd_os_result__(id, true, 1);
    expect(await os).toBe(1);
  });

  test("a remote top document never asks for the app token", () => {
    const { win, raw } = evalShim(undefined, undefined, {
      protocol: "https:",
      origin: "https://evil.example",
      href: "https://evil.example/",
    });
    g.window = win;
    expect(raw).toEqual([]);
    win.__tynd__.os_call("app", "getInfo", {});
    expect(raw.length).toBe(1);
    expect(raw[0]?.startsWith(`${TOKEN}{`)).toBe(true);
  });
});
